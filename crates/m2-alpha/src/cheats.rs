//! The test cheat menu (F7, with the developer's keys: `--dev`, or a test
//! launch): any hand weapon, any vehicle and any support item, each from
//! the game's own lists, and the usual helpers (god mode, infinite ammo,
//! money, a teleport).
//!
//! Every list comes from the data, nothing is named here:
//!
//! - weapons: the weapon templates with an `Equipment` carry class (what
//!   [`Game::give`] can put in her hands) and the thrown ones, by template
//!   name;
//! - vehicles: the templates whose physics component is a car's, a
//!   tank's, a boat's or a helicopter's and whose model the archive holds,
//!   not ruins ([`crate::mission::template_models`]): parked by her as the
//!   vehicles she starts with are, with their template's handling;
//! - support: the scripts' support catalogue (`tSupportData`: the supply
//!   drops, the vehicle deliveries and the air and artillery strikes),
//!   stocked free into her support menu or called in at once.
//!
//! Provenance: the lists are the data's; the menu, its key and its
//! helpers are invented for testing (the game has no such menu on PC).

use m2_engine::world::{Templates, VehicleKind, member_stem};
use m2_engine::{CatalogueEntry, weapons::Arms};
use m2_math::glam::Vec3;

use crate::game::Game;
use crate::mission::MissionLink;

/// How far ahead "Teleport ahead" puts her (metres; invented).
pub const AHEAD: f32 = 100.0;

/// What a money button gives: cash and fuel (invented).
pub const CASH: f64 = 1_000_000.0;
pub const FUEL: f64 = 1_000.0;

/// A vehicle template to park by her.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VehicleLine {
    /// Its template name as the database writes it.
    pub name: String,
    /// `car`, `tank`, `boat` or `helicopter`.
    pub kind: &'static str,
}

/// A support item of the scripts' catalogue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupportLine {
    /// Its key in `tSupportData` (`"clusterbomb"`).
    pub key: String,
    /// Readable, from the string table.
    pub name: String,
    /// `sType`: Supply, Light, Heavy, Civilian, Heli, Boat or Airstrike.
    pub kind: String,
}

/// The menu's pages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Weapons,
    Vehicles,
    Support,
    Player,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Weapons, Tab::Vehicles, Tab::Support, Tab::Player];

    pub fn name(self) -> &'static str {
        match self {
            Tab::Weapons => "Weapons",
            Tab::Vehicles => "Vehicles",
            Tab::Support => "Support",
            Tab::Player => "Player",
        }
    }
}

/// What stays on while the menu is shut.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Toggles {
    /// Her health kept whole.
    pub god: bool,
    /// Her clips, reserves and explosives kept full.
    pub ammo: bool,
}

/// The menu up: its lists and what the last action said.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CheatScreen {
    pub tab: Tab,
    pub weapons: Vec<String>,
    pub vehicles: Vec<VehicleLine>,
    pub support: Vec<SupportLine>,
    pub toggles: Toggles,
    /// Only the lines containing this (any case).
    pub filter: String,
    pub said: Option<String>,
}

/// What the player does in the menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheatAction {
    Tab(Tab),
    Filter(String),
    Weapon(String),
    Vehicle(String),
    /// Stock one, free, into her support menu.
    StockSupport(String),
    /// Stock one and call it in now: she takes up its designator.
    CallSupport(String),
    God(bool),
    Ammo(bool),
    Money,
    Heal,
    TeleportAhead,
    TeleportObjective,
    Close,
}

impl CheatScreen {
    /// The menu's lists as the game's data and the scripts' catalogue
    /// give them now (the support list is empty until the scripts run).
    pub fn read(game: &Game, mission: Option<&MissionLink>) -> Self {
        let scene = &game.scene;
        Self {
            weapons: scene
                .arms()
                .map(|a| weapons(a, &carried(game)))
                .unwrap_or_default(),
            vehicles: match (&scene.world, &scene.catalogue) {
                (Some(w), Some(c)) => vehicles(&w.index.template_db, &c.vehicles),
                _ => Vec::new(),
            },
            support: mission.map(MissionLink::cheat_support).unwrap_or_default(),
            ..Self::default()
        }
    }

    /// How long its lists are, for the log.
    pub fn counts(&self) -> String {
        format!(
            "{} weapons, {} vehicles, {} support items",
            self.weapons.len(),
            self.vehicles.len(),
            self.support.len()
        )
    }
}

/// What a cheat did: a line for the menu, and whether the menu closes
/// (to take up a designator, or after a teleport).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub said: String,
    pub close: bool,
}

impl Applied {
    fn say(said: String) -> Self {
        Self { said, close: false }
    }
}

/// Does `action` (any but the menu's own: [`CheatAction::Tab`],
/// [`CheatAction::Filter`], [`CheatAction::Close`]).
pub fn apply(
    game: &mut Game,
    mission: Option<&mut MissionLink>,
    toggles: &mut Toggles,
    action: CheatAction,
) -> Applied {
    use CheatAction as A;
    let on_off = |on: bool| if on { "on" } else { "off" };
    match action {
        A::Tab(_) | A::Filter(_) | A::Close => Applied::say(String::new()),
        A::Weapon(name) => Applied::say(if game.give(&name) {
            format!("Given: {name}.")
        } else {
            format!("Could not give {name}.")
        }),
        A::Vehicle(name) => {
            let models = vehicle_models(game, &name);
            Applied::say(if models.is_empty() {
                format!("No model for {name}.")
            } else {
                game.park_more(models);
                format!("{name} is on its way: it parks by her once loaded.")
            })
        }
        A::StockSupport(key) => {
            let stocked = mission.is_some_and(|m| m.cheat_support_item(&key, false));
            Applied::say(if stocked {
                format!("Stocked: {key}. Call it in from the support menu.")
            } else {
                format!("Could not stock {key}: the mission scripts aren't running.")
            })
        }
        A::CallSupport(key) => {
            if mission.is_some_and(|m| m.cheat_support_item(&key, true)) {
                Applied {
                    said: format!("{key} called in."),
                    close: true,
                }
            } else {
                Applied::say(format!(
                    "{key} was not called in (the scripts take calls on foot, on land)."
                ))
            }
        }
        A::God(on) => {
            toggles.god = on;
            Applied::say(format!("God mode {}.", on_off(on)))
        }
        A::Ammo(on) => {
            toggles.ammo = on;
            Applied::say(format!("Infinite ammo {}.", on_off(on)))
        }
        A::Heal => {
            game.combat.hp = game.combat.max_hp;
            Applied::say("Healed.".to_owned())
        }
        A::Money => Applied::say(match mission {
            Some(m) => {
                m.cheat_money(CASH, FUEL);
                "Money and fuel given.".to_owned()
            }
            None => "No money until the mission scripts run.".to_owned(),
        }),
        A::TeleportAhead | A::TeleportObjective => {
            let Some(p) = game.scene.player.as_ref() else {
                return Applied::say("No one to move.".to_owned());
            };
            let (from, yaw) = (p.character.position, p.yaw);
            let on_foot = game.on_foot && p.driving.is_none();
            let to = if action == A::TeleportAhead {
                Some(ahead(from, yaw, AHEAD))
            } else {
                let markers = mission.map(|m| m.view().markers).unwrap_or_default();
                nearest(from, markers.iter().map(|m| m.at))
            };
            match to {
                Some(to) if on_foot => {
                    game.teleported(from, to);
                    if let Some(p) = game.scene.player.as_mut() {
                        p.drop_at(to);
                    }
                    Applied {
                        said: format!("Teleported from {from:.1} to {to:.1}."),
                        close: true,
                    }
                }
                Some(_) => Applied::say("Teleports work on foot only.".to_owned()),
                None => Applied::say("No objective marker to go to.".to_owned()),
            }
        }
    }
}

/// The names of the thrown weapons she has a slot for.
fn carried(game: &Game) -> Vec<&str> {
    game.combat
        .throwables
        .iter()
        .flatten()
        .map(|t| t.name.as_str())
        .collect()
}

/// The models vehicle template `name` parks as.
fn vehicle_models(game: &Game, name: &str) -> Vec<CatalogueEntry> {
    let scene = &game.scene;
    match (&scene.world, &scene.catalogue) {
        (Some(w), Some(c)) => {
            crate::mission::template_models(name, &w.index.template_db, &c.vehicles)
        }
        _ => Vec::new(),
    }
}

/// Vehicles `--cheat-check` parks: this many, spread evenly through the
/// list, the first of each kind among them.
const CHECK_VEHICLES: usize = 8;
/// Seconds `--cheat-check` waits, once she is ashore, before it starts,
/// and then lets the world run before it counts what came (invented).
const CHECK_SETTLE: f32 = 10.0;
pub const CHECK_RUN: f32 = 60.0;
/// Seconds after the teleport its landing is looked at (before the
/// autoplay walks her on).
const TELEPORT_LOOK: f32 = 5.0;

/// `--cheat-check`: every cheat tried through [`apply`], as the menu does,
/// once she is ashore and the scripts run: every weapon given, a few
/// vehicles of each kind parked, every support item stocked and the first
/// strike called in, money given, god mode and infinite ammo kept up, and
/// a teleport ahead; then what came of it counted (`cheat-check:` lines).
#[derive(Debug, Default)]
pub struct Check {
    ashore_for: f32,
    /// Seconds since the cheats were tried, once they are.
    since: Option<f32>,
    toggles: Toggles,
    /// The vehicles parked, by their label ([`member_stem`] of the name).
    vehicles: Vec<String>,
    /// Her cash before the money, and the teleport's target.
    cash: f64,
    target: Option<Vec3>,
    /// Where she stood [`TELEPORT_LOOK`] seconds after it.
    landed: Option<Vec3>,
    /// The support's counts when the strike was called.
    support: (u32, u32, u32),
    pub lines: Vec<String>,
}

impl Check {
    /// One tick: tries the cheats once she has been ashore a while.
    /// Whether it is over.
    pub fn step(&mut self, game: &mut Game, link: &mut MissionLink, ashore: bool, dt: f32) -> bool {
        keep_up(game, self.toggles);
        if let Some(t) = &mut self.since {
            *t += dt;
            if *t >= TELEPORT_LOOK && self.landed.is_none() {
                self.landed = game.scene.player.as_ref().map(|p| p.character.position);
            }
            if *t >= CHECK_RUN {
                self.count(game, link);
                return true;
            }
            return false;
        }
        self.ashore_for = if ashore && link.running() {
            self.ashore_for + dt
        } else {
            0.0
        };
        if self.ashore_for >= CHECK_SETTLE {
            self.try_all(game, link);
            self.since = Some(0.0);
        }
        false
    }

    fn line(&mut self, line: String) {
        eprintln!("cheat-check: {line}");
        self.lines.push(line);
    }

    fn try_all(&mut self, game: &mut Game, link: &mut MissionLink) {
        let screen = CheatScreen::read(game, Some(link));
        self.line(format!("lists: {}", screen.counts()));
        let mut apply = |game: &mut Game, link: &mut MissionLink, action| {
            apply(game, Some(link), &mut self.toggles, action)
        };
        // Every weapon.
        let failed: Vec<&String> = screen
            .weapons
            .iter()
            .filter(|w| {
                apply(game, link, CheatAction::Weapon((*w).clone()))
                    .said
                    .starts_with("Could")
            })
            .collect();
        let given = screen.weapons.len() - failed.len();
        let failed = format!("{failed:?}");
        // Vehicles: the first of each kind, and the rest spread evenly.
        let mut picks: Vec<&VehicleLine> = Vec::new();
        for kind in ["car", "tank", "boat", "helicopter"] {
            picks.extend(screen.vehicles.iter().find(|v| v.kind == kind));
        }
        let step = (screen.vehicles.len() / CHECK_VEHICLES).max(1);
        for v in screen.vehicles.iter().step_by(step) {
            if picks.len() >= CHECK_VEHICLES {
                break;
            }
            if !picks.contains(&v) {
                picks.push(v);
            }
        }
        let mut sent = 0;
        for v in &picks {
            if !apply(game, link, CheatAction::Vehicle(v.name.clone()))
                .said
                .starts_with("No")
            {
                sent += 1;
                self.vehicles
                    .push(member_stem(&v.name).to_ascii_lowercase());
            }
        }
        let picked: Vec<String> = picks
            .iter()
            .map(|v| format!("{} ({})", v.name, v.kind))
            .collect();
        // Every support item stocked; the first strike called in.
        let stocked = screen
            .support
            .iter()
            .filter(|s| {
                apply(game, link, CheatAction::StockSupport(s.key.clone()))
                    .said
                    .starts_with("Stocked")
            })
            .count();
        let c = &link.counts;
        self.support = (c.arrived, c.flown, c.burst);
        let strike = screen.support.iter().find(|s| s.kind == "Airstrike");
        let called = strike.map(|s| {
            let done = apply(game, link, CheatAction::CallSupport(s.key.clone()));
            (s.key.clone(), done.close)
        });
        // Money, god mode, infinite ammo, a teleport.
        self.cash = link.resources().map_or(0.0, |r| r.0);
        apply(game, link, CheatAction::Money);
        apply(game, link, CheatAction::God(true));
        apply(game, link, CheatAction::Ammo(true));
        let from = game.scene.player.as_ref().map(|p| p.character.position);
        let teleport = apply(game, link, CheatAction::TeleportAhead);
        self.target = from
            .filter(|_| teleport.close)
            .zip(game.scene.player.as_ref())
            .map(|(_, p)| p.character.position);
        self.line(format!(
            "weapons: {given} of {} given; failed {failed}",
            screen.weapons.len()
        ));
        self.line(format!(
            "vehicles: {sent} of {} sent: {}",
            picks.len(),
            picked.join(", ")
        ));
        self.line(format!(
            "support: {stocked} of {} stocked; strike called {called:?}",
            screen.support.len()
        ));
        self.line(format!("teleport: {}", teleport.said));
    }

    fn count(&mut self, game: &mut Game, link: &mut MissionLink) {
        let parked: Vec<String> = game
            .scene
            .vehicles
            .iter()
            .filter(|v| v.id.is_some())
            .map(|v| v.spec.label.to_ascii_lowercase())
            .collect();
        let came = self.vehicles.iter().filter(|l| parked.contains(l)).count();
        self.line(format!(
            "vehicles parked: {came} of {}",
            self.vehicles.len()
        ));
        let cash = link.resources().map_or(0.0, |r| r.0);
        self.line(format!("cash: {:.0} -> {cash:.0}", self.cash));
        let c = &link.counts;
        let (a, f, b) = self.support;
        self.line(format!(
            "support since the call: {} arrived, {} flown over, {} burst",
            c.arrived - a,
            c.flown - f,
            c.burst - b
        ));
        // God mode and infinite ammo: hurt and emptied, then kept up.
        game.combat.hp = 1.0;
        game.combat.rifle_rounds.clip = 0;
        game.combat.grenades = 0;
        keep_up(game, self.toggles);
        let combat = &game.combat;
        self.line(format!(
            "god: health {:.0} of {:.0}; ammo: clip {} of {}, grenades {}",
            combat.hp, combat.max_hp, combat.rifle_rounds.clip, combat.rifle.clip, combat.grenades
        ));
        let at = self.landed;
        if let (Some(to), Some(at)) = (self.target, at) {
            let off = Vec3::new(at.x - to.x, 0.0, at.z - to.z).length();
            self.line(format!(
                "teleport: {TELEPORT_LOOK:.0} s on she stands {off:.1} m (across) from the target, at {at:.1}"
            ));
        }
    }
}

/// Whether `line` passes the menu's filter.
pub fn shown(line: &str, filter: &str) -> bool {
    let filter = filter.trim();
    filter.is_empty()
        || line
            .to_ascii_lowercase()
            .contains(&filter.to_ascii_lowercase())
}

/// Every hand weapon's template name, each once (any case), sorted: the
/// guns with a carry class, and the thrown weapons named in `carried`
/// (those she has a slot for: the grenade, C4, the proximity mine; not the
/// designators the support hands her, nor the AI's grenade).
pub fn weapons(arms: &Arms, carried: &[&str]) -> Vec<String> {
    let mut names: Vec<String> = arms
        .weapons
        .values()
        .filter(|w| w.class.is_some())
        .map(|w| w.name.clone())
        .chain(
            arms.throwables
                .values()
                .filter(|t| carried.iter().any(|c| c.eq_ignore_ascii_case(&t.name)))
                .map(|t| t.name.clone()),
        )
        .filter(|n| !n.trim().is_empty())
        .collect();
    sort_unique(&mut names);
    names
}

/// Every vehicle template that parks: a car's, tank's, boat's or
/// helicopter's physics and a model the archive holds, not a ruin, and
/// one [`crate::mission::template_models`] finds a model for. Sorted by
/// name, each once.
pub fn vehicles(templates: &Templates, catalogue: &[CatalogueEntry]) -> Vec<VehicleLine> {
    let mut out: Vec<VehicleLine> = templates
        .infos
        .iter()
        .filter(|(_, info)| info.model.is_some())
        .filter_map(|(id, info)| {
            let kind = match info.vehicle? {
                VehicleKind::Car => "car",
                VehicleKind::Tank => "tank",
                VehicleKind::Boat => "boat",
                VehicleKind::Helicopter => "helicopter",
                VehicleKind::Unknown => return None,
            };
            let name = templates.names.get(id)?;
            Some(VehicleLine {
                name: name.clone(),
                kind,
            })
        })
        .filter(|v| !crate::mission::is_ruin(templates, &v.name))
        .filter(|v| !crate::mission::template_models(&v.name, templates, catalogue).is_empty())
        .collect();
    // By name in any case, then as written: the templates come in hash
    // order, and the same name spelt twice keeps the same one.
    out.sort_by_key(|v| (v.name.to_ascii_lowercase(), v.name.clone()));
    out.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    out
}

fn sort_unique(names: &mut Vec<String>) {
    names.sort_by_key(|n| (n.to_ascii_lowercase(), n.clone()));
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
}

/// Keeps up what `toggles` turns on, each tick: her health whole, her
/// guns' clips and reserves and her explosives full.
pub fn keep_up(game: &mut Game, toggles: Toggles) {
    let c = &mut game.combat;
    if toggles.god {
        c.hp = c.max_hp;
        c.down = 0.0;
    }
    if toggles.ammo {
        c.rifle_rounds.clip = c.rifle.clip;
        c.rifle_rounds.stored = c.rifle_rounds.stored.max(c.rifle.spare);
        if let Some(r) = &mut c.launcher_rounds {
            r.clip = c.launcher.clip;
            r.stored = r.stored.max(c.launcher.spare);
        }
        let most = |i: usize| c.throwables.get(i).and_then(|t| t.as_ref()).map(|t| t.max);
        if let Some(m) = most(0) {
            c.grenades = c.grenades.max(m);
        }
        if let Some(m) = most(1) {
            c.charges = c.charges.max(m);
        }
        if let Some(m) = most(2) {
            c.mines = c.mines.max(m);
        }
    }
}

/// Where "Teleport ahead" puts her: `distance` metres along `yaw` (the
/// camera's heading, radians as [`crate::player::Player::yaw`]) from
/// `from`, high above, to come down on whatever is there.
pub fn ahead(from: Vec3, yaw: f32, distance: f32) -> Vec3 {
    let (s, c) = yaw.sin_cos();
    from + Vec3::new(s, 0.0, c) * distance + Vec3::Y * 50.0
}

/// Of `markers`, the nearest to `from` (on the ground plane).
pub fn nearest(from: Vec3, markers: impl IntoIterator<Item = Vec3>) -> Option<Vec3> {
    markers.into_iter().min_by(|a, b| {
        let d = |p: &Vec3| (p.x - from.x).hypot(p.z - from.z);
        d(a).total_cmp(&d(b))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_engine::world::TemplateInfo;

    fn info(model: Option<u32>, vehicle: Option<VehicleKind>) -> TemplateInfo {
        TemplateInfo {
            model,
            vehicle,
            engine: None,
            person: false,
            human: false,
        }
    }

    #[test]
    fn the_filter_matches_any_case_and_an_empty_one_all() {
        assert!(shown("Rocket Launcher (AT)", ""));
        assert!(shown("Rocket Launcher (AT)", "  "));
        assert!(shown("Rocket Launcher (AT)", "launcher"));
        assert!(shown("Rocket Launcher (AT)", " ROCKET "));
        assert!(!shown("Rocket Launcher (AT)", "shotgun"));
    }

    #[test]
    fn vehicles_are_the_templates_with_vehicle_physics_and_a_model() {
        let mut t = Templates::default();
        let add = |t: &mut Templates, id: u32, name: &str, i: TemplateInfo| {
            t.names.insert(id, name.to_owned());
            t.infos.insert(id, i);
            t.by_name.entry(name.to_ascii_lowercase()).or_insert(i);
        };
        add(
            &mut t,
            0x8000_0001,
            "M151 (VZ)",
            info(Some(1), Some(VehicleKind::Car)),
        );
        add(
            &mut t,
            0x8000_0002,
            "T72 (VZ)",
            info(Some(2), Some(VehicleKind::Tank)),
        );
        add(
            &mut t,
            0x8000_0003,
            "Patrol Boat",
            info(Some(3), Some(VehicleKind::Boat)),
        );
        add(
            &mut t,
            0x8000_0004,
            "Mi26 (PMC)",
            info(Some(4), Some(VehicleKind::Helicopter)),
        );
        // No model the archive holds, no physics, unknown physics, a ruin,
        // and the same name twice: none listed, or listed once.
        add(
            &mut t,
            0x8000_0005,
            "Ghost Jeep",
            info(None, Some(VehicleKind::Car)),
        );
        add(&mut t, 0x8000_0006, "Crate", info(Some(6), None));
        add(
            &mut t,
            0x8000_0007,
            "Odd",
            info(Some(7), Some(VehicleKind::Unknown)),
        );
        add(
            &mut t,
            0x8000_0008,
            "M151_Ruin",
            info(Some(8), Some(VehicleKind::Car)),
        );
        t.ruined.insert(0x8000_0008);
        add(
            &mut t,
            0x8000_0009,
            "m151 (vz)",
            info(Some(1), Some(VehicleKind::Car)),
        );
        let v = vehicles(&t, &[]);
        let names: Vec<(&str, &str)> = v.iter().map(|l| (l.name.as_str(), l.kind)).collect();
        assert_eq!(
            names,
            [
                ("M151 (VZ)", "car"),
                ("Mi26 (PMC)", "helicopter"),
                ("Patrol Boat", "boat"),
                ("T72 (VZ)", "tank"),
            ]
        );
    }

    #[test]
    fn ahead_follows_the_heading_and_starts_high() {
        let at = ahead(Vec3::new(10.0, 5.0, 20.0), 0.0, 100.0);
        assert!((at - Vec3::new(10.0, 55.0, 120.0)).length() < 1e-3);
        let at = ahead(Vec3::ZERO, std::f32::consts::FRAC_PI_2, 100.0);
        assert!((at - Vec3::new(100.0, 50.0, 0.0)).length() < 1e-3);
    }

    #[test]
    fn the_nearest_marker_is_taken_on_the_ground_plane() {
        let from = Vec3::ZERO;
        let far_low = Vec3::new(30.0, 0.0, 0.0);
        let near_high = Vec3::new(10.0, 500.0, 0.0);
        assert_eq!(nearest(from, [far_low, near_high]), Some(near_high));
        assert_eq!(nearest(from, []), None);
    }
}
