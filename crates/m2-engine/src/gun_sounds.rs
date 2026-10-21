//! Which cue a gun plays when it fires: the front end's `Sounds` table.
//!
//! A gun's template names a `SoundSet` (its `SoundEffect`'s field: the
//! carbine's and the combat rifle's `01b47c45`, the assault rifles'
//! `7473fe68`). The `Sounds` table (an animation table, `9cd06f1d`, in
//! the resident blocks of `vz.wad` and `shell.wad` alike) has the columns
//! `SoundSet`, `EventHandle`, `Param` and `SoundID`: a row gives the cue
//! (`SoundID`, a cue of the world's cue tables, plain or a mix of several
//! records, [`m2_assets::MixedCue`]) a sound set plays for an event. The
//! events named so far are [`FIRE`] (`weapon_fire`: the gun's own bank,
//! `wpn_combatrifle`'s three loud shots for the carbine), [`FIRE_NPC`]
//! (`weapon_fire_npc`: a cue of the resident `wpn_shared` bank),
//! [`DRY_FIRE`] (`weapon_dryfire`), `reload` and `stow`. A gun that spins
//! (the mounted miniguns' set `624c0986`) has no shot: it plays [`LOOP`]
//! (`weapon_loop`, `wpn_minigun_fire_loop`) or [`LOOP_NPC`]
//! (`weapon_loop_npc`) for as long as it fires, and [`FIRE_TAIL`]
//! (`weapon_fire_tail`, a row most sets have) when it stops. The grapple's
//! set answers its own events (`grapple_fire`, `grapple_attach`,
//! `grapple_reel`, `grapple_reel_loop`, `grapple_loop`,
//! `grapple_release`); [`GRAPPLE_FIRE`] is its shot. [`EQUIP_PICKUP`]
//! (`a639af36`, a cue of the `ui_hud` bank on most sets) plays as the gun
//! is taken up (name found by hashing guesses, gap-list sweep: one hit in
//! 22,243 tries).
//! `Param` is [`ANY`] on most rows. The assault rifles' NPC shot (set
//! `7473fe68`, which hashes as `assaultrifle`) has six rows keyed
//! `variant_1` to `variant_6` instead, giving three cues (records 11, 12
//! and 13 of the `wpn_shared` bank) in turn: 1 and 4, 2 and 5, 3 and 6
//! share a cue. No other gun set has such rows. [`GunCues::shot_by`]
//! picks a shooter's variant from a number of theirs.
//!
//! # Provenance
//!
//! From samples: the table's columns are its own names; the events'
//! names are found by hashing guesses (round 19 named the loop, tail and
//! grapple events so) (`weapon_fire` hashes to the
//! carbine's row whose cue is its bank's loudest three-variant record);
//! that a gun plays `weapon_fire` when the hero fires and
//! `weapon_fire_npc` when anyone else does is inferred from the names. The
//! `variant_N` names are found the same way (all six rows). That each
//! shooter keeps one variant (so soldiers sound apart, rather than each
//! shot picking anew) is inferred from there being six variants for three
//! cues; how the game numbers a shooter's variant is invented here (a
//! number of theirs, modulo the variants the set has). Not checked
//! against the game's code.
//!
//! 7 of the 66 firing and reload cues the table names find no sound
//! (gap-list sweep, `firecues` example): no cue table in `vz.wad`,
//! `shell.wad`, `English.wad` or `Loading.wad` has a record for them.
//! They are two `weapon_fire_tail` cues shared by 27 sets (272 guns), the
//! miniguns' `weapon_loop_npc`, the RPG family's and the shotgun's
//! `reload`, and the shots of two sets no weapon template uses. In the
//! data they sound of nothing; the alpha plays nothing for them either
//! (a shot of theirs would fall back to the rifle's, but no gun fires
//! those two sets).

use std::collections::HashMap;
use std::io::{Read, Seek};

use m2_assets::animation_table::{ANIMATION_TABLE_TYPE, ANY, AnimationTable};
use m2_assets::name_hash;

use crate::assets::{AssetSource, Install};

/// The `Sounds` table's name hash.
pub const SOUNDS_TABLE: u32 = 0x9cd0_6f1d;
/// The events a gun's sound set answers.
pub const FIRE: &str = "weapon_fire";
pub const FIRE_NPC: &str = "weapon_fire_npc";
pub const DRY_FIRE: &str = "weapon_dryfire";
pub const LOOP: &str = "weapon_loop";
pub const LOOP_NPC: &str = "weapon_loop_npc";
pub const FIRE_TAIL: &str = "weapon_fire_tail";
pub const GRAPPLE_FIRE: &str = "grapple_fire";
/// Its gun reloaded: a row of the [`ANY`] set's, keyed by the set in
/// `Param` ([`GunCues::reload`]).
pub const RELOAD: &str = "reload";
/// Its gun taken up off the ground: an interface sound (a cue of the
/// `ui_hud` bank on most sets; named by hashing, gap-list sweep).
pub const EQUIP_PICKUP: &str = "equip_pickup";

/// Each sound set's cue per event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GunCues {
    cues: HashMap<(u32, u32), u32>,
    /// A set's cue per event keyed `variant_1`, `variant_2`, ... in
    /// order, for sets and events with no plain row.
    variants: HashMap<(u32, u32), Vec<u32>>,
    /// The cue each set plays as its gun is reloaded: the [`ANY`] set's
    /// `reload` rows, which key the set in `Param` (22 sets).
    reloads: HashMap<u32, u32>,
}

/// The most `variant_N` keys looked for.
const MOST_VARIANTS: u32 = 16;

/// The `Param` hash of `variant_<n>`.
fn variant_key(n: u32) -> u32 {
    name_hash(&format!("variant_{n}"))
}

impl GunCues {
    /// From the table's rows (`SoundSet`, `EventHandle`, `Param`,
    /// `SoundID`): of several rows for a set and event, the one whose
    /// `Param` is [`ANY`], else the first. Rows naming no set (the
    /// [`ANY`] set's, which key the set in `Param` instead) are left out.
    /// Rows keyed `variant_1`, `variant_2`, ... (up to the first number
    /// missing) are kept apart too, as the set's variants for the event.
    pub fn from_rows(rows: impl IntoIterator<Item = [u32; 4]>) -> Self {
        let keys: HashMap<u32, u32> = (1..=MOST_VARIANTS).map(|n| (variant_key(n), n)).collect();
        let mut cues = HashMap::new();
        let mut plain = std::collections::HashSet::new();
        let mut numbered: HashMap<(u32, u32), HashMap<u32, u32>> = HashMap::new();
        let reload = name_hash(RELOAD);
        let mut reloads = HashMap::new();
        for [set, event, param, cue] in rows {
            if set == ANY && event == reload && param != ANY && cue != ANY && cue != 0 {
                reloads.entry(param).or_insert(cue);
            }
            if set == ANY || cue == ANY || cue == 0 {
                continue;
            }
            if let Some(&n) = keys.get(&param) {
                numbered
                    .entry((set, event))
                    .or_default()
                    .entry(n)
                    .or_insert(cue);
            }
            if param == ANY {
                if plain.insert((set, event)) {
                    cues.insert((set, event), cue);
                }
            } else if !plain.contains(&(set, event)) {
                cues.entry((set, event)).or_insert(cue);
            }
        }
        let variants = numbered
            .into_iter()
            .filter(|(key, _)| !plain.contains(key))
            .filter_map(|(key, by_n)| {
                let list: Vec<u32> = (1..=MOST_VARIANTS)
                    .map_while(|n| by_n.get(&n).copied())
                    .collect();
                (!list.is_empty()).then_some((key, list))
            })
            .collect();
        Self {
            cues,
            variants,
            reloads,
        }
    }

    /// From the parsed table; `None` if it lacks one of its columns.
    pub fn from_table(table: &AnimationTable) -> Option<Self> {
        let cols = [
            table.column("SoundSet")?,
            table.column("EventHandle")?,
            table.column("Param")?,
            table.column("SoundID")?,
        ];
        Some(Self::from_rows((0..table.rows()).filter_map(|r| {
            Some([
                table.cell(r, cols[0])?,
                table.cell(r, cols[1])?,
                table.cell(r, cols[2])?,
                table.cell(r, cols[3])?,
            ])
        })))
    }

    /// Reads the table from the install's world archive.
    pub fn read(install: &Install) -> Result<Self, String> {
        let mut source = install.open_world().map_err(|e| e.to_string())?;
        Self::from_source(&mut source)
    }

    /// Reads the table from an archive holding it.
    pub fn from_source<R: Read + Seek>(source: &mut AssetSource<R>) -> Result<Self, String> {
        let data = source
            .asset(SOUNDS_TABLE, ANIMATION_TABLE_TYPE)
            .map_err(|e| e.to_string())?;
        let table = AnimationTable::parse(&data).map_err(|e| e.to_string())?;
        Self::from_table(&table).ok_or_else(|| "the Sounds table lacks a column".to_owned())
    }

    /// The cue sound set `set` plays for `event` ([`FIRE`], ...).
    pub fn cue(&self, set: u32, event: &str) -> Option<u32> {
        self.cues.get(&(set, name_hash(event))).copied()
    }

    /// The cue a gun of sound set `set` fires with: the hero's
    /// ([`FIRE`]) or anyone else's ([`FIRE_NPC`], else [`FIRE`]); the
    /// grapple's [`GRAPPLE_FIRE`].
    pub fn shot(&self, set: u32, npc: bool) -> Option<u32> {
        if npc {
            self.cue(set, FIRE_NPC).or_else(|| self.cue(set, FIRE))
        } else {
            self.cue(set, FIRE)
        }
        .or_else(|| self.cue(set, GRAPPLE_FIRE))
    }

    /// [`shot`](Self::shot), for a shooter numbered `shooter`: where the
    /// event has `variant_N` rows, the shooter's variant (`shooter`
    /// modulo their count; invented); else the plain shot.
    pub fn shot_by(&self, set: u32, npc: bool, shooter: u32) -> Option<u32> {
        let event = name_hash(if npc { FIRE_NPC } else { FIRE });
        match self.variants.get(&(set, event)) {
            Some(list) => list.get(shooter as usize % list.len()).copied(),
            None => self.shot(set, npc),
        }
    }

    /// The `variant_N` cues sound set `set` has for `event`, in order:
    /// empty for most.
    pub fn variants(&self, set: u32, event: &str) -> &[u32] {
        self.variants
            .get(&(set, name_hash(event)))
            .map_or(&[], Vec::as_slice)
    }

    /// Set and event pairs with `variant_N` rows.
    pub fn varied(&self) -> usize {
        self.variants.len()
    }

    /// The cue a spinning gun of sound set `set` loops while it fires
    /// ([`LOOP`], or [`LOOP_NPC`] for anyone but the hero, else
    /// [`LOOP`]): `None` for a gun with a shot of its own
    /// ([`shot`](Self::shot)).
    pub fn spin(&self, set: u32, npc: bool) -> Option<u32> {
        if self.shot(set, npc).is_some() {
            return None;
        }
        if npc {
            self.cue(set, LOOP_NPC).or_else(|| self.cue(set, LOOP))
        } else {
            self.cue(set, LOOP)
        }
    }

    /// The cue sound set `set` plays as its gun is reloaded, if the table
    /// gives one.
    pub fn reload(&self, set: u32) -> Option<u32> {
        self.reloads.get(&set).copied()
    }

    /// Sound sets with a reload cue.
    pub fn reloading(&self) -> usize {
        self.reloads.len()
    }

    /// Sound sets with a cue for `event`.
    pub fn sets_with(&self, event: &str) -> usize {
        let e = name_hash(event);
        self.cues.keys().filter(|(_, ev)| *ev == e).count()
    }

    pub fn is_empty(&self) -> bool {
        self.cues.is_empty()
    }

    /// The sets and events (name hashes) that play cue `cue`, sorted; a
    /// reload's event is [`RELOAD`]'s.
    pub fn uses_of(&self, cue: u32) -> Vec<(u32, u32)> {
        let reload = name_hash(RELOAD);
        let mut out: Vec<(u32, u32)> = self
            .cues
            .iter()
            .filter(|(_, c)| **c == cue)
            .map(|(&k, _)| k)
            .chain(
                self.variants
                    .iter()
                    .filter(|(_, v)| v.contains(&cue))
                    .map(|(&k, _)| k),
            )
            .chain(
                self.reloads
                    .iter()
                    .filter(|(_, c)| **c == cue)
                    .map(|(&set, _)| (set, reload)),
            )
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Every cue a gun plays as it fires (shots, the NPC shot's variants,
    /// loops and tails), is taken up and reloads, each once, in order: what to read ahead so that
    /// no first shot waits on its cue.
    pub fn firing_cues(&self) -> Vec<u32> {
        let events: Vec<u32> = [
            FIRE,
            FIRE_NPC,
            LOOP,
            LOOP_NPC,
            FIRE_TAIL,
            GRAPPLE_FIRE,
            EQUIP_PICKUP,
        ]
        .iter()
        .map(|e| name_hash(e))
        .collect();
        let mut out: Vec<u32> = self
            .cues
            .iter()
            .filter(|((_, e), _)| events.contains(e))
            .map(|(_, &c)| c)
            .chain(
                self.variants
                    .iter()
                    .filter(|((_, e), _)| events.contains(e))
                    .flat_map(|(_, v)| v.iter().copied()),
            )
            .chain(self.reloads.values().copied())
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// How loud every gun's shot strikes once levelled: the loudest tenth of
/// a second of its sounds, RMS (`-9 dBFS`, the median of the hero's 27
/// shot cues; invented).
pub const SHOT_PUNCH: f32 = 0.355;
/// The most a shot is turned up or down by levelling (invented: 6 dB).
const SHOT_LEVEL_MOST: f32 = 2.0;

/// The gain a gun's shot plays at, from its cue's `volume` and its
/// sounds' `punch` ([`crate::audio::KeySounds::punch`]): the cue's
/// volume, levelled so its punch is [`SHOT_PUNCH`], by no more than 6 dB
/// either way.
///
/// The cues' volumes are all 0.95 to 1, while their sounds strike 9.5 dB
/// apart at worst: the carbine's (`wpn_combatrifle`) at -6.6 dBFS and
/// 562 rounds a minute against the assault rifle's mixed cue at -13 and
/// 450, 6 dB louder held down (playtest 5: "way louder than other
/// guns"). How the game balances them (its mixer, categories or the
/// records' fields not yet read) is not decoded; the levelling is
/// invented.
pub fn shot_gain(volume: f32, punch: f32) -> f32 {
    if punch <= 0.0 {
        return volume;
    }
    volume * (SHOT_PUNCH / punch).clamp(1.0 / SHOT_LEVEL_MOST, SHOT_LEVEL_MOST)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The event names hash to the table's `EventHandle`s (read from a
    // legally installed copy's table: the carbine's rows).
    #[test]
    fn the_event_names_hash_to_the_tables_handles() {
        assert_eq!(name_hash(FIRE), 0x8f87_f89c);
        assert_eq!(name_hash(FIRE_NPC), 0x3842_7a98);
        assert_eq!(name_hash(DRY_FIRE), 0x6eea_9c0f);
        assert_eq!(name_hash("reload"), 0x8190_ec52);
        assert_eq!(name_hash("stow"), 0xd8db_0afe);
        assert_eq!(name_hash(LOOP), 0xa670_3286);
        assert_eq!(name_hash(LOOP_NPC), 0x9987_da7e);
        assert_eq!(name_hash(FIRE_TAIL), 0x4504_59d9);
        assert_eq!(name_hash(GRAPPLE_FIRE), 0x70d9_68fd);
    }

    // The minigun's rows as the table has them: a loop, an NPC loop, a
    // tail and a dry fire, no shot. The grapple's shot is its own event.
    #[test]
    fn a_minigun_loops_and_the_grapple_fires_its_own_event() {
        let (minigun, grapple, carbine) = (0x624c_0986, 0x25f7_53ca, 0x01b4_7c45);
        let cues = GunCues::from_rows([
            [minigun, 0x9987_da7e, ANY, 0x2b5d_d674],
            [minigun, 0xa670_3286, ANY, 0xa51b_442a],
            [minigun, 0x4504_59d9, ANY, 0x0bb3_1676],
            [grapple, 0x70d9_68fd, ANY, 0x6513_964d],
            [carbine, 0x8f87_f89c, ANY, 0x33a0_71cc],
        ]);
        assert_eq!(cues.shot(minigun, false), None);
        assert_eq!(cues.spin(minigun, false), Some(0xa51b_442a));
        assert_eq!(cues.spin(minigun, true), Some(0x2b5d_d674));
        assert_eq!(cues.cue(minigun, FIRE_TAIL), Some(0x0bb3_1676));
        assert_eq!(cues.shot(grapple, false), Some(0x6513_964d));
        // A gun with a shot does not spin.
        assert_eq!(cues.spin(carbine, false), None);
    }

    // The carbine's rows as the table has them, and the assault rifle's
    // NPC shot, keyed by `Param` only.
    #[test]
    fn a_set_fires_its_own_cue_and_its_npc_cue() {
        let (carbine, ak) = (0x01b4_7c45, 0x7473_fe68);
        let cues = GunCues::from_rows([
            [carbine, 0x3842_7a98, ANY, 0x1fc5_1b48],
            [carbine, 0x8f87_f89c, ANY, 0x33a0_71cc],
            [carbine, 0x6eea_9c0f, ANY, 0x18be_fe36],
            [ANY, 0x8190_ec52, carbine, 0x176b_614b],
            [ak, 0x3842_7a98, 0x6333_f2bc, 0xd8ce_1427],
            [ak, 0x3842_7a98, 0x3d31_7853, 0x8827_8c4f],
            [ak, 0x8f87_f89c, ANY, 0x4988_08cf],
        ]);
        assert_eq!(cues.shot(carbine, false), Some(0x33a0_71cc));
        assert_eq!(cues.shot(carbine, true), Some(0x1fc5_1b48));
        assert_eq!(cues.cue(carbine, DRY_FIRE), Some(0x18be_fe36));
        // The reload row keys the set in `Param`: not read as a set's,
        // but as the set's reload.
        assert_eq!(cues.cue(carbine, "reload"), None);
        assert_eq!(cues.reload(carbine), Some(0x176b_614b));
        assert_eq!(cues.reload(ak), None);
        assert_eq!(cues.reloading(), 1);
        // Of the `Param`-keyed rows, the first.
        assert_eq!(cues.shot(ak, true), Some(0xd8ce_1427));
        assert_eq!(cues.shot(ak, false), Some(0x4988_08cf));
        // An unknown set fires nothing.
        assert_eq!(cues.shot(0x1234_5678, false), None);
        assert_eq!(cues.sets_with(FIRE), 2);
    }

    // The carbine's punch (-6.6 dBFS) turned down to the shared level, the
    // assault rifle's (-13) up, a suppressed pistol's (-18.4) by no more
    // than 6 dB; a silent cue keeps its volume.
    #[test]
    fn shots_are_levelled_by_their_punch_within_6_db() {
        let db = |x: f32| 10f32.powf(x / 20.0);
        let heard = |volume: f32, punch: f32| shot_gain(volume, punch) * punch;
        let carbine = heard(0.96, db(-6.6));
        let rifle = heard(1.0, db(-13.0));
        assert!((carbine - 0.96 * SHOT_PUNCH).abs() < 1e-3, "{carbine}");
        assert!((rifle - SHOT_PUNCH).abs() < 1e-3, "{rifle}");
        assert!((shot_gain(1.0, db(-18.4)) - 2.0).abs() < 1e-6);
        assert!((shot_gain(1.0, db(0.0)) - 0.5).abs() < 1e-6);
        assert_eq!(shot_gain(0.9, 0.0), 0.9);
    }

    // A cue's users: the tail shared by two sets, a reload, a variant.
    #[test]
    fn a_cues_users_are_its_sets_and_events() {
        let (minigun, carbine, ak) = (0x624c_0986, 0x01b4_7c45, 0x7473_fe68);
        let tail = name_hash(FIRE_TAIL);
        assert_eq!(tail, 0x4504_59d9);
        let cues = GunCues::from_rows([
            [minigun, tail, ANY, 0x0bb3_1676],
            [carbine, tail, ANY, 0x0bb3_1676],
            [ANY, name_hash(RELOAD), carbine, 0x176b_614b],
            [ak, name_hash(FIRE_NPC), variant_key(1), 0x33a0_71cc],
        ]);
        let mut both = vec![(carbine, tail), (minigun, tail)];
        both.sort_unstable();
        assert_eq!(cues.uses_of(0x0bb3_1676), both);
        assert_eq!(cues.uses_of(0x176b_614b), [(carbine, name_hash(RELOAD))]);
        assert_eq!(cues.uses_of(0x33a0_71cc), [(ak, name_hash(FIRE_NPC))]);
        assert!(cues.uses_of(1).is_empty());
    }

    // What is read ahead: every firing event's cue, the NPC variants and
    // the reloads, once each; not the dry fire.
    #[test]
    fn firing_cues_are_the_shots_loops_tails_variants_and_reloads() {
        let (minigun, carbine, ak) = (0x624c_0986, 0x01b4_7c45, 0x7473_fe68);
        let cues = GunCues::from_rows([
            [minigun, 0x9987_da7e, ANY, 0x2b5d_d674],
            [minigun, 0xa670_3286, ANY, 0xa51b_442a],
            [minigun, 0x4504_59d9, ANY, 0x0bb3_1676],
            [carbine, 0x8f87_f89c, ANY, 0x33a0_71cc],
            [carbine, 0x3842_7a98, ANY, 0x1fc5_1b48],
            [carbine, 0x6eea_9c0f, ANY, 0x18be_fe36],
            [ANY, 0x8190_ec52, carbine, 0x176b_614b],
            [ak, 0x3842_7a98, variant_key(1), 0x33a0_71cc],
            [ak, 0x3842_7a98, variant_key(2), 0xd8ce_1427],
        ]);
        assert_eq!(
            cues.firing_cues(),
            [
                0x0bb3_1676,
                0x176b_614b,
                0x1fc5_1b48,
                0x2b5d_d674,
                0x33a0_71cc,
                0xa51b_442a,
                0xd8ce_1427
            ]
        );
    }

    // A plain row overrides the `Param`-keyed rows before it.
    #[test]
    fn a_plain_row_wins_over_keyed_ones() {
        let cues = GunCues::from_rows([[1, 2, 7, 10], [1, 2, ANY, 20], [1, 2, 8, 30]]);
        assert_eq!(cues.cues.get(&(1, 2)), Some(&20));
    }

    // The `Param` values of the assault rifles' NPC shot (read from a
    // legally installed copy's table) hash as `variant_1` to `variant_6`.
    #[test]
    fn the_assault_rifles_params_hash_as_variants() {
        let rows = [
            0x6333_f2bc,
            0x3d31_7853,
            0x5b2f_68f6,
            0x6540_c1d5,
            0x3b3e_4120,
            0xe53b_7b27,
        ];
        for (n, row) in (1..).zip(rows) {
            assert_eq!(variant_key(n), row);
        }
        assert_eq!(name_hash("assaultrifle"), 0x7473_fe68);
    }

    // The table's six rows give three cues in turn; a shooter keeps
    // theirs; the hero's shot and other sets are untouched.
    #[test]
    fn shooters_take_their_variant_of_the_npc_shot() {
        let ak = 0x7473_fe68;
        let (a, b, c) = (0xd8ce_1427, 0x8827_8c4f, 0xfe2a_84a8);
        let cues = GunCues::from_rows([
            [ak, 0x3842_7a98, 0x6333_f2bc, a],
            [ak, 0x3842_7a98, 0x3d31_7853, b],
            [ak, 0x3842_7a98, 0x5b2f_68f6, c],
            [ak, 0x3842_7a98, 0x6540_c1d5, a],
            [ak, 0x3842_7a98, 0x3b3e_4120, b],
            [ak, 0x3842_7a98, 0xe53b_7b27, c],
            [ak, 0x8f87_f89c, ANY, 0x4988_08cf],
            [9, 0x3842_7a98, ANY, 0x77],
        ]);
        assert_eq!(cues.variants(ak, FIRE_NPC), &[a, b, c, a, b, c]);
        assert_eq!(cues.varied(), 1);
        let heard: Vec<_> = (0..6).map(|s| cues.shot_by(ak, true, s).unwrap()).collect();
        assert_eq!(heard, [a, b, c, a, b, c]);
        assert_eq!(cues.shot_by(ak, true, 7), Some(b));
        // Without a shooter, the first, as before.
        assert_eq!(cues.shot(ak, true), Some(a));
        assert_eq!(cues.shot_by(ak, false, 3), Some(0x4988_08cf));
        assert_eq!(cues.shot_by(9, true, 3), Some(0x77));
    }

    // A plain row for the event leaves its variants unused, and the
    // numbering stops at the first number missing.
    #[test]
    fn variants_need_no_plain_row_and_no_gap() {
        let cues = GunCues::from_rows([
            [1, 2, variant_key(1), 10],
            [1, 2, ANY, 20],
            [3, 2, variant_key(1), 30],
            [3, 2, variant_key(3), 40],
        ]);
        assert_eq!(cues.variants.get(&(1, 2)), None);
        assert_eq!(cues.variants.get(&(3, 2)), Some(&vec![30]));
        assert_eq!(cues.varied(), 1);
    }
}
