//! The `Sound` and `VO` natives that set the sound engine's state:
//! category volumes and pitches, the low-pass filter, pauses, filters,
//! user music, and the bank and test calls.
//!
//! From the 360 code (natives `0x82340CD0`..`0x82349620`, listed in
//! `crate::api`): each reads its arguments with the engine's readers
//! (`0x82308518` a string, hashed; `0x823082B8` a number; `0x82308230` a
//! boolean) and answers a single nil (`0x82206F20`) when one is missing,
//! changing nothing. Otherwise the setters answer nothing and, when the
//! sound system is up (`0x82DE3ED0`), store into it: the low-pass cutoff
//! at mixer +0x130, its settings at +0x128/+0x12C, the system pause at
//! +0x15C (and the sound manager's +0x30), the pause filter, the stream
//! block dumping flag, a message filter per hashed name, and the
//! categories' volume (kind 1) and pitch (kind 7) through `0x8236C678`.
//! The hostility decay rate starts at 1.0 (`0x82B7686C`).
//!
//! [`SoundSettings`] keeps what the scripts set. Invented: an unset
//! category reads volume and pitch 1.0 (the 360 asks its mixer, whose
//! defaults weren't traced); the game's audio doesn't read these settings
//! yet.

use mlua::{MultiValue, Value};
use std::collections::BTreeMap;

use mlua::{Lua, Table};

use crate::api::{Command, ScriptWorld};
use crate::script_hash::engine_hash;

/// Argument `i` (from 1) as the engine's string reader takes it, hashed.
pub fn hash_arg(args: &[Value], i: usize) -> Option<u32> {
    let v = args.get(i.checked_sub(1)?)?;
    let s = match v {
        Value::String(s) => s.as_bytes().to_vec(),
        Value::Integer(i) => i.to_string().into_bytes(),
        Value::Number(n) => format!("{}", n).into_bytes(),
        _ => return None,
    };
    Some(engine_hash(&s))
}

/// Argument `i` (from 1) as a number.
pub fn num_arg(args: &[Value], i: usize) -> Option<f32> {
    let v = args.get(i.checked_sub(1)?)?;
    match v {
        Value::Integer(i) => Some(*i as f32),
        Value::Number(n) => Some(*n as f32),
        Value::String(s) => {
            let text = s.to_string_lossy().trim().to_string();
            text.parse::<f64>().ok().map(|n| n as f32)
        }
        _ => None,
    }
}

/// Argument `i` (from 1) as a boolean (only a boolean).
pub fn bool_arg(args: &[Value], i: usize) -> Option<bool> {
    match args.get(i.checked_sub(1)?)? {
        Value::Boolean(b) => Some(*b),
        _ => None,
    }
}

pub fn nil() -> MultiValue {
    MultiValue::from_vec(vec![Value::Nil])
}

pub fn none() -> MultiValue {
    MultiValue::new()
}

pub fn number(n: f32) -> MultiValue {
    MultiValue::from_vec(vec![Value::Number(n as f64)])
}

pub fn yes_no(b: bool) -> MultiValue {
    MultiValue::from_vec(vec![Value::Boolean(b)])
}

/// The sound engine's state the scripts set.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundSettings {
    pub user_music_overridden: bool,
    pub force_action_transition: bool,
    pub fade_categories_cleared: u32,
    pub hostility_decay_rate: f32,
    pub low_pass_cutoff: f32,
    pub low_pass_settings: (f32, f32),
    pub stream_block_dumping: bool,
    pub ambience_silenced: bool,
    pub message_filters: BTreeMap<u32, bool>,
    pub pause_filter: bool,
    pub system_pause: bool,
    pub category_volumes: BTreeMap<u32, f32>,
    pub category_pitches: BTreeMap<u32, f32>,
    pub removed_playlists: Vec<u32>,
}

impl Default for SoundSettings {
    fn default() -> Self {
        Self {
            user_music_overridden: false,
            force_action_transition: false,
            fade_categories_cleared: 0,
            hostility_decay_rate: 1.0,
            low_pass_cutoff: 0.0,
            low_pass_settings: (0.0, 0.0),
            stream_block_dumping: false,
            ambience_silenced: false,
            message_filters: BTreeMap::new(),
            pause_filter: false,
            system_pause: false,
            category_volumes: BTreeMap::new(),
            category_pitches: BTreeMap::new(),
            removed_playlists: Vec::new(),
        }
    }
}

pub fn override_user_music(s: &mut SoundSettings, _args: &[Value]) -> MultiValue {
    s.user_music_overridden = true;
    none()
}

pub fn restore_user_music(s: &mut SoundSettings, _args: &[Value]) -> MultiValue {
    s.user_music_overridden = false;
    none()
}

pub fn force_action_transition(s: &mut SoundSettings, _args: &[Value]) -> MultiValue {
    s.force_action_transition = true;
    none()
}

pub fn clear_fade_categories(s: &mut SoundSettings, _args: &[Value]) -> MultiValue {
    s.fade_categories_cleared += 1;
    none()
}

pub fn set_hostility_decay_rate_music(s: &mut SoundSettings, args: &[Value]) -> MultiValue {
    match num_arg(args, 1) {
        Some(v) => {
            s.hostility_decay_rate = v;
            none()
        }
        None => nil(),
    }
}

pub fn set_low_pass_filter(s: &mut SoundSettings, args: &[Value]) -> MultiValue {
    match num_arg(args, 1) {
        Some(v) => {
            s.low_pass_cutoff = v;
            none()
        }
        None => nil(),
    }
}

pub fn set_low_pass_filter_settings(s: &mut SoundSettings, args: &[Value]) -> MultiValue {
    let v1 = num_arg(args, 1);
    let v2 = num_arg(args, 2);
    match (v1, v2) {
        (Some(a), Some(b)) => {
            s.low_pass_settings = (a, b);
            none()
        }
        _ => nil(),
    }
}

pub fn set_stream_block_dumping(s: &mut SoundSettings, args: &[Value]) -> MultiValue {
    match bool_arg(args, 1) {
        Some(v) => {
            s.stream_block_dumping = v;
            none()
        }
        None => nil(),
    }
}

pub fn silence_ambience(s: &mut SoundSettings, args: &[Value]) -> MultiValue {
    match bool_arg(args, 1) {
        Some(v) => {
            s.ambience_silenced = v;
            none()
        }
        None => nil(),
    }
}

pub fn set_message_filtering(s: &mut SoundSettings, args: &[Value]) -> MultiValue {
    let h = hash_arg(args, 1);
    let b = bool_arg(args, 2);
    match (h, b) {
        (Some(hash), Some(flag)) => {
            s.message_filters.insert(hash, flag);
            none()
        }
        _ => nil(),
    }
}

pub fn set_pause_filter(s: &mut SoundSettings, args: &[Value]) -> MultiValue {
    match bool_arg(args, 1) {
        Some(v) => {
            s.pause_filter = v;
            none()
        }
        None => nil(),
    }
}

pub fn set_system_pause(s: &mut SoundSettings, args: &[Value]) -> MultiValue {
    match bool_arg(args, 1) {
        Some(v) => {
            s.system_pause = v;
            none()
        }
        None => nil(),
    }
}

pub fn set_category_volume(s: &mut SoundSettings, args: &[Value]) -> MultiValue {
    let h = hash_arg(args, 1);
    let v = num_arg(args, 2);
    match (h, v) {
        (Some(hash), Some(vol)) => {
            s.category_volumes.insert(hash, vol);
            none()
        }
        _ => nil(),
    }
}

pub fn set_category_pitch(s: &mut SoundSettings, args: &[Value]) -> MultiValue {
    let h = hash_arg(args, 1);
    let v = num_arg(args, 2);
    match (h, v) {
        (Some(hash), Some(pitch)) => {
            s.category_pitches.insert(hash, pitch);
            none()
        }
        _ => nil(),
    }
}

pub fn get_category_volume(s: &SoundSettings, args: &[Value]) -> MultiValue {
    match hash_arg(args, 1) {
        Some(hash) => {
            let vol = s.category_volumes.get(&hash).copied().unwrap_or(1.0);
            number(vol)
        }
        None => nil(),
    }
}

pub fn get_category_pitch(s: &SoundSettings, args: &[Value]) -> MultiValue {
    match hash_arg(args, 1) {
        Some(hash) => {
            let pitch = s.category_pitches.get(&hash).copied().unwrap_or(1.0);
            number(pitch)
        }
        None => nil(),
    }
}

pub fn remove_music_source_playlist(s: &mut SoundSettings, args: &[Value]) -> MultiValue {
    match hash_arg(args, 1) {
        Some(hash) => {
            s.removed_playlists.push(hash);
            none()
        }
        None => nil(),
    }
}

/// The first `count` arguments as hashed names, all needed.
pub fn bank_args(args: &[Value], count: usize) -> Option<Vec<u32>> {
    (1..=count)
        .map(|i| hash_arg(args, i))
        .collect::<Option<Vec<_>>>()
}

type Handler = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

/// `w.sound` through a setter.
macro_rules! setter {
    ($f:ident) => {
        |w: &mut ScriptWorld, _: &Lua, a: &[Value]| Ok($f(&mut w.sound, a))
    };
}

/// The text of argument `i` (from 1) as the engine's `0x823084A8` reads
/// it: a string, or a number as Lua 5.1 writes it.
fn text_arg(args: &[Value], i: usize) -> Option<String> {
    match args.get(i.checked_sub(1)?)? {
        Value::String(s) => Some(s.to_string_lossy()),
        Value::Integer(n) => Some(n.to_string()),
        Value::Number(n) => Some(format!("{n}")),
        _ => None,
    }
}

/// `Sound.Test*Sound(cue)` (`0x82340CD0`, `0x82340D98`): the sound
/// engine's test calls, on the local player; nil without a cue.
fn test_cue(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(cue) = text_arg(a, 1) else {
        return Ok(nil());
    };
    w.commands.push(Command::CueSound { at: w.player, cue });
    Ok(none())
}

fn test_stop(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    let Some(cue) = text_arg(a, 1) else {
        return Ok(nil());
    };
    w.commands.push(Command::StopSound { at: w.player, cue });
    Ok(none())
}

/// The bank calls (`LoadBank` takes three names, `UnloadBank` two, the
/// sound and wave bank calls one): nil unless they are all there.
fn banks<const N: usize>(_: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    Ok(match bank_args(a, N) {
        Some(_) => none(),
        None => nil(),
    })
}

/// `Sound.RegisterReadyCallback(f)` (`0x82343D18`): the callback the
/// sound system calls once it is ready (`0x8235D3B8`, at once if it
/// already is). The port's sound is always ready: it is called on the
/// next tick, as the bank callbacks are (inferred timing).
fn ready_callback(w: &mut ScriptWorld, _: &Lua, a: &[Value]) -> mlua::Result<MultiValue> {
    if let Some(Value::Function(f)) = a.first() {
        w.asset_loads.push((f.clone(), Value::Nil));
    }
    Ok(none())
}

/// `VO.CancelAll()` (`0x82349398`): every line stops (`0x82277D30`
/// cancels from the last one down), and it answers true.
fn vo_cancel_all(w: &mut ScriptWorld, _: &Lua, _: &[Value]) -> mlua::Result<MultiValue> {
    let hush = w.vo.cancel_all();
    w.commands.extend(hush);
    Ok(yes_no(true))
}

pub(crate) fn install(
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Handler) -> mlua::Result<()>,
) -> mlua::Result<()> {
    let sound = lib("Sound")?;
    let handlers: [(&str, Handler); 31] = [
        ("OverrideUserMusic", setter!(override_user_music)),
        ("RestoreUserMusic", setter!(restore_user_music)),
        ("ForceActionTransition", setter!(force_action_transition)),
        ("ClearFadeCategories", setter!(clear_fade_categories)),
        (
            "SetHostilityDecayRateMusic",
            setter!(set_hostility_decay_rate_music),
        ),
        ("SetLowPassFilter", setter!(set_low_pass_filter)),
        (
            "SetLowPassFilterSettings",
            setter!(set_low_pass_filter_settings),
        ),
        ("SetStreamBlockDumping", setter!(set_stream_block_dumping)),
        ("SilenceAmbience", setter!(silence_ambience)),
        ("SetMessageFiltering", setter!(set_message_filtering)),
        ("SetPauseFilter", setter!(set_pause_filter)),
        ("SetSystemPause", setter!(set_system_pause)),
        ("SetCategoryVolume", setter!(set_category_volume)),
        ("SetCategoryPitch", setter!(set_category_pitch)),
        (
            "RemoveMusicSourcePlaylist",
            setter!(remove_music_source_playlist),
        ),
        ("GetCategoryVolume", |w, _, a| {
            Ok(get_category_volume(&w.sound, a))
        }),
        ("GetCategoryPitch", |w, _, a| {
            Ok(get_category_pitch(&w.sound, a))
        }),
        // The cue's longest length from the audio engine; the port's cues
        // have none, so 0, the 360's answer with its sound off (invented).
        ("GetMaxDuration", |_, _, a| {
            Ok(if hash_arg(a, 1).is_some() {
                number(0.0)
            } else {
                nil()
            })
        }),
        // The music's action-level lock byte (+0xD911); the port's music
        // doesn't lock it.
        ("IsActionLevelLockedMusic", |_, _, _| Ok(yes_no(false))),
        ("CloseStreamFile", |_, _, a| {
            Ok(if text_arg(a, 1).is_some() {
                none()
            } else {
                nil()
            })
        }),
        // (object, cue): a pause message for the cue on the object.
        ("PauseSound", |_, _, a| {
            Ok(if hash_arg(a, 2).is_some() {
                none()
            } else {
                nil()
            })
        }),
        ("TestCueSound", test_cue),
        ("TestStopSound", test_stop),
        ("TestPauseSound", |_, _, a| {
            Ok(if hash_arg(a, 1).is_some() {
                none()
            } else {
                nil()
            })
        }),
        ("LoadBank", banks::<3>),
        ("UnloadBank", banks::<2>),
        ("LoadSoundBank", banks::<1>),
        ("LoadWaveBank", banks::<1>),
        ("UnloadSoundBank", banks::<1>),
        ("UnloadWaveBank", banks::<1>),
        ("RegisterReadyCallback", ready_callback),
    ];
    for (name, f) in handlers {
        add(&sound, name, f)?;
    }
    let vo = lib("VO")?;
    add(&vo, "CancelAll", vo_cancel_all)?;
    // Pause and Unpause share one native (`0x82349400`), as PauseAll and
    // UnpauseAll do (`0x82349550`): each sends the same message (kind 2)
    // for the lines it finds and answers true. Not passed to the game.
    for name in ["Pause", "Unpause", "PauseAll", "UnpauseAll"] {
        add(&vo, name, |_, _, _| Ok(yes_no(true)))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    #[test]
    fn categories_keep_what_the_scripts_set() {
        let lua = Lua::new();
        let music = Value::String(lua.create_string("Music").unwrap());
        let mut s = SoundSettings::default();
        assert_eq!(s.hostility_decay_rate, 1.0);
        assert!(set_category_volume(&mut s, &[music.clone(), Value::Number(0.25)]).is_empty());
        let v = get_category_volume(&s, std::slice::from_ref(&music));
        assert_eq!(v.into_iter().collect::<Vec<_>>(), [Value::Number(0.25)]);
        let other = Value::String(lua.create_string("Sfx").unwrap());
        let v = get_category_pitch(&s, &[other]);
        assert_eq!(v.into_iter().collect::<Vec<_>>(), [Value::Number(1.0)]);
        // A number names the category its text does.
        set_category_pitch(&mut s, &[Value::Integer(5), Value::Number(0.5)]);
        assert_eq!(s.category_pitches[&engine_hash(b"5")], 0.5);
    }

    #[test]
    fn a_missing_argument_answers_nil_and_changes_nothing() {
        let lua = Lua::new();
        let music = Value::String(lua.create_string("Music").unwrap());
        let mut s = SoundSettings::default();
        let r = set_category_volume(&mut s, std::slice::from_ref(&music));
        assert_eq!(r.into_iter().collect::<Vec<_>>(), [Value::Nil]);
        assert!(s.category_volumes.is_empty());
        assert_eq!(
            set_low_pass_filter_settings(&mut s, &[Value::Number(1.0)]).len(),
            1
        );
        assert_eq!(s.low_pass_settings, (0.0, 0.0));
        assert!(
            set_low_pass_filter_settings(&mut s, &[Value::Number(1.0), Value::Integer(2)])
                .is_empty()
        );
        assert_eq!(s.low_pass_settings, (1.0, 2.0));
        assert_eq!(bool_arg(&[Value::Integer(1)], 1), None);
        assert_eq!(set_pause_filter(&mut s, &[Value::Integer(1)]).len(), 1);
        assert!(get_category_volume(&s, &[]).len() == 1);
    }

    #[test]
    fn filters_playlists_and_banks_go_by_hash() {
        let lua = Lua::new();
        let combat = Value::String(lua.create_string("Combat").unwrap());
        let mut s = SoundSettings::default();
        assert!(set_message_filtering(&mut s, &[combat.clone(), Value::Boolean(true)]).is_empty());
        assert!(s.message_filters[&engine_hash(b"Combat")]);
        remove_music_source_playlist(&mut s, std::slice::from_ref(&combat));
        assert_eq!(s.removed_playlists, [engine_hash(b"Combat")]);
        let a = Value::String(lua.create_string("a").unwrap());
        assert_eq!(bank_args(std::slice::from_ref(&a), 2), None);
        assert_eq!(
            bank_args(&[a.clone(), combat], 2),
            Some(vec![engine_hash(b"a"), engine_hash(b"Combat")])
        );
    }
}
