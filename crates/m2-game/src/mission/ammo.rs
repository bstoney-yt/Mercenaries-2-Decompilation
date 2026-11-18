//! Thrown ammunition as objects of the scripts' world: the C4 she sticks
//! somewhere (`C4 Projectile`, whose script is `C4`, 0xcaf30da8) and the
//! Beacon Designator's beacon (`ammo_designator_beacon`, whose linked
//! child `ammo_designator_beacon_light` runs `beacon`, 0x673d7049).
//!
//! From samples (the two scripts' listings): `OnActivate(guid)` plays the
//! object's material animation (`global_weapon_c4land_30thsec`,
//! `global_weapon_beacon`; looped) and one second later
//! (`Event.TimerRelative { 1 }`) cues `wpn_bomb_timer_01_armed` on the
//! object; `OnDeactivate(guid)` stops the animation and the cue and
//! deletes the timer. The two scripts differ only in the animation.
//!
//! From the 360 code: a spawn (0x82409AA8) instances its template
//! (0x82408C30), and the instancing spawns the child templates the
//! template's link records name and links them to it
//! ([`crate::links::template_children`]); every object then runs its
//! template's object script as it comes in (`OnActivate`, 0x82401528)
//! and as it goes (`OnDeactivate`, 0x82401618).
//!
//! Not traced: the order the parent's and child's `OnActivate` run in
//! (here the parent's first); that the children go with the parent
//! (inferred: the light is part of the beacon); and whether a stuck
//! charge's children follow it (they stay where it is).

use super::{Mission, object_script};
use crate::api::{Object, ScriptWorld};
use crate::links::Link;

/// `C4`'s module hash: the script of `C4 Projectile`.
pub const C4: u32 = 0xcaf3_0da8;

/// `beacon`'s module hash: the script of `ammo_designator_beacon_light`.
pub const BEACON: u32 = 0x673d_7049;

/// How deep children of children are spawned: a guard against a template
/// naming itself (none does).
const MAX_DEPTH: u32 = 4;

/// Spawns `template` at `at` into `w` with its child templates, linked
/// to it; every guid made, parent first.
fn instance(w: &mut ScriptWorld, template: &str, at: [f32; 3], depth: u32, made: &mut Vec<u32>) {
    let guid = w.spawn_guid();
    w.insert(guid, Object::new(template, at));
    let lower = template.to_ascii_lowercase();
    w.note_script(guid, &lower);
    made.push(guid);
    children(w, guid, &lower, at, depth, made);
}

/// Spawns the child templates of `template` (lower case) for `guid`,
/// linked to it.
fn children(
    w: &mut ScriptWorld,
    guid: u32,
    template: &str,
    at: [f32; 3],
    depth: u32,
    made: &mut Vec<u32>,
) {
    if depth >= MAX_DEPTH {
        return;
    }
    let children = w
        .template_children
        .get(template)
        .cloned()
        .unwrap_or_default();
    for (key, child) in children {
        let first = made.len();
        instance(w, &child, at, depth + 1, made);
        let c = made[first];
        w.links.insert(Link {
            parent: guid,
            hardpoint: c,
            key,
            child: c,
        });
    }
}

/// `guid` and everything linked under it, parent first.
fn family(w: &ScriptWorld, guid: u32) -> Vec<u32> {
    let mut all = vec![guid];
    let mut i = 0;
    while i < all.len() && all.len() < 64 {
        let kids: Vec<u32> = w.links.children(all[i]).collect();
        for k in kids {
            if !all.contains(&k) {
                all.push(k);
            }
        }
        i += 1;
    }
    all
}

impl Mission {
    /// Makes thrown ammunition (`template`: `"C4 Projectile"`,
    /// `"ammo_designator_beacon"`) an object of the scripts' world at
    /// `at`, with the child templates its template spawns, and runs their
    /// object scripts' `OnActivate` now. Its guid; move it with
    /// [`ammo_moved`](Self::ammo_moved), take it away with
    /// [`ammo_gone`](Self::ammo_gone).
    pub fn ammo_spawned(&mut self, template: &str, at: [f32; 3]) -> u32 {
        let mut made = Vec::new();
        instance(&mut self.world.borrow_mut(), template, at, 0, &mut made);
        self.activate_now(&made);
        made[0]
    }

    /// The child templates `template` spawns with it, made for `guid`
    /// (already in the scripts' world: a thrown designator, whose own
    /// script the support manager is called on its own), their scripts
    /// run now ([`crate::links::template_children`]).
    pub(super) fn spawn_children(&mut self, guid: u32, template: &str, at: [f32; 3]) {
        let mut made = Vec::new();
        let lower = template.to_ascii_lowercase();
        children(&mut self.world.borrow_mut(), guid, &lower, at, 0, &mut made);
        self.activate_now(&made);
    }

    /// Runs `made`'s object scripts' `OnActivate` now rather than at the
    /// next tick, in order.
    fn activate_now(&mut self, made: &[u32]) {
        let runs = {
            let mut w = self.world.borrow_mut();
            let mut runs = Vec::new();
            for g in made {
                if let Some(i) = w.script_spawns.iter().position(|s| s.0 == *g) {
                    runs.push(w.script_spawns.remove(i));
                }
            }
            runs
        };
        for (guid, script, data) in runs {
            if let Some(Err(f)) = object_script(self.host.lua(), script, true, guid, data) {
                self.log.push(format!("  {f} (ammo {guid:#x})"));
            }
        }
        if !made.is_empty() {
            self.settle("ammo");
        }
    }

    /// Thrown ammunition `guid` is at `at`: it and its children.
    pub fn ammo_moved(&mut self, guid: u32, at: [f32; 3]) {
        let mut w = self.world.borrow_mut();
        for g in family(&w, guid) {
            if let Some(o) = w.objects.get_mut(&g) {
                o.position = at;
            }
        }
    }

    /// Thrown ammunition `guid` went off or went: it and its children
    /// leave the scripts' world, their scripts told (`OnDeactivate`, at
    /// the next tick).
    pub fn ammo_gone(&mut self, guid: u32) {
        let mut w = self.world.borrow_mut();
        let all = family(&w, guid);
        for g in all.into_iter().rev() {
            w.remove_object(g);
            w.links.remove(g);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Command;
    use crate::mission::{GameData, OBJECT_SCRIPTS};

    fn mission() -> Mission {
        let mut data = GameData::default();
        data.template_scripts
            .insert("c4 projectile".into(), (C4, None));
        data.template_scripts
            .insert("ammo_designator_beacon_light".into(), (BEACON, None));
        data.template_children.insert(
            "ammo_designator_beacon".into(),
            vec![(0x1234, "ammo_designator_beacon_light".into())],
        );
        Mission::new(data, "Mattias", [0.0; 3]).unwrap()
    }

    /// The scripts' own shape (`C4` and `beacon` differ only in the
    /// animation's name), for tests without the game's data.
    fn script(module: &str, anim: &str) -> String {
        format!(
            r#"{module} = {{}}
            local tEvents = {{}}
            function {module}.OnActivate(self)
                Object.PlayMaterialAnimation(self, "{anim}", true)
                tEvents[self] = Event.Create(Event.TimerRelative, {{ 1 }},
                    Sound.CueSound, {{ self, "wpn_bomb_timer_01_armed" }})
            end
            function {module}.OnDeactivate(self)
                Object.StopMaterialAnimation(self, "{anim}")
                Sound.StopSound(self, "wpn_bomb_timer_01_armed")
                Event.Delete(tEvents[self])
                tEvents[self] = nil
            end"#
        )
    }

    #[test]
    fn both_scripts_are_run() {
        for h in [C4, BEACON] {
            assert!(OBJECT_SCRIPTS.iter().any(|(_, s)| *s == h), "{h:#x}");
        }
    }

    fn cues(commands: &[Command], guid: u32) -> (usize, usize) {
        let mut n = (0, 0);
        for c in commands {
            match c {
                Command::CueSound { at: Some(g), cue }
                    if *g == guid && cue == "wpn_bomb_timer_01_armed" =>
                {
                    n.0 += 1
                }
                Command::StopSound { at: Some(g), cue }
                    if *g == guid && cue == "wpn_bomb_timer_01_armed" =>
                {
                    n.1 += 1
                }
                _ => {}
            }
        }
        n
    }

    // Stuck C4 blinks at once and beeps one second on, until it goes off.
    #[test]
    fn a_charge_blinks_then_beeps_until_it_goes() {
        let mut m = mission();
        m.lua()
            .load(script("C4", "global_weapon_c4land_30thsec"))
            .exec()
            .unwrap();
        m.start();
        m.take_commands();
        let g = m.ammo_spawned("C4 Projectile", [1.0, 2.0, 3.0]);
        let first = m.take_commands();
        assert!(first.iter().any(|c| matches!(c,
            Command::MaterialAnimation { guid, name, looped: true }
                if *guid == g && name == "global_weapon_c4land_30thsec")));
        assert_eq!(cues(&first, g), (0, 0), "no beep before a second");
        m.tick(0.5);
        assert_eq!(cues(&m.take_commands(), g), (0, 0));
        m.tick(0.6);
        assert_eq!(cues(&m.take_commands(), g), (1, 0), "the beep starts");
        m.tick(2.0);
        assert_eq!(cues(&m.take_commands(), g), (0, 0), "once, not again");
        m.ammo_gone(g);
        m.tick(0.1);
        let last = m.take_commands();
        assert_eq!(cues(&last, g), (0, 1), "the beep stops as it goes");
        assert!(!m.world().borrow().objects.contains_key(&g));
    }

    // Gone before its second, it never beeps (the timer is deleted).
    #[test]
    fn a_charge_gone_at_once_never_beeps() {
        let mut m = mission();
        m.lua()
            .load(script("C4", "global_weapon_c4land_30thsec"))
            .exec()
            .unwrap();
        m.start();
        let g = m.ammo_spawned("C4 Projectile", [0.0; 3]);
        m.tick(0.25);
        m.ammo_gone(g);
        m.tick(0.25);
        m.take_commands();
        m.tick(2.0);
        assert_eq!(cues(&m.take_commands(), g), (0, 0));
    }

    // The beacon brings its light (the template's child), linked to it;
    // the light runs `beacon`, and goes with the beacon.
    #[test]
    fn the_beacon_brings_its_light_which_runs_beacon() {
        let mut m = mission();
        m.lua()
            .load(script("beacon", "global_weapon_beacon"))
            .exec()
            .unwrap();
        m.start();
        m.take_commands();
        let b = m.ammo_spawned("ammo_designator_beacon", [5.0, 0.0, 5.0]);
        let light = {
            let w = m.world().borrow();
            let l = w.links.linked(b, 0x1234).expect("the light is linked");
            assert_eq!(w.objects[&l].name, "ammo_designator_beacon_light");
            l
        };
        assert!(m.take_commands().iter().any(|c| matches!(c,
            Command::MaterialAnimation { guid, name, .. }
                if *guid == light && name == "global_weapon_beacon")));
        m.ammo_moved(b, [6.0, 1.0, 6.0]);
        assert_eq!(m.world().borrow().objects[&light].position, [6.0, 1.0, 6.0]);
        m.tick(1.1);
        assert_eq!(cues(&m.take_commands(), light), (1, 0));
        m.ammo_gone(b);
        m.tick(0.1);
        assert_eq!(cues(&m.take_commands(), light), (0, 1));
        let w = m.world().borrow();
        assert!(!w.objects.contains_key(&b) && !w.objects.contains_key(&light));
        assert_eq!(w.links.children(b).count(), 0);
    }

    // Thrown as her designator (the support manager's object), the beacon
    // brings its light all the same, which follows it to where it lies.
    #[test]
    fn a_thrown_beacon_designator_brings_its_light() {
        let mut m = mission();
        m.lua()
            .load(script("beacon", "global_weapon_beacon"))
            .exec()
            .unwrap();
        m.start();
        m.take_commands();
        let b = m.designator_thrown(1, "ammo_designator_beacon", [0.0; 3]);
        let light = m
            .world()
            .borrow()
            .links
            .linked(b, 0x1234)
            .expect("its light");
        assert!(m.take_commands().iter().any(|c| matches!(c,
            Command::MaterialAnimation { guid, .. } if *guid == light)));
        m.designator_settled(b, [3.0, 0.0, 4.0]);
        assert_eq!(m.world().borrow().objects[&light].position, [3.0, 0.0, 4.0]);
        // A smoke grenade has no children.
        let s = m.designator_thrown(1, "Smoke Grenade Projectile", [0.0; 3]);
        assert_eq!(m.world().borrow().links.children(s).count(), 0);
    }
}
