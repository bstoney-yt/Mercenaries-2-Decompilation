//! The scripts' yes/no questions about the build and the player's
//! options: `Sys.IsFinalConfig`, `Sys.NoHud`, `Sys.IsDemoMode` and the
//! rest of the `Sys` flags.
//!
//! From the 360 code (natives `0x82344F70`..`0x82346250`): each getter
//! pushes one boolean and answers one value. They read three kinds of
//! flag:
//!
//! - **Constants.** `IsFinalConfig` is always true and `NoHud` always
//!   false in the retail build.
//! - **Developer switches** read from the debug INI by `0x822193D8`
//!   (keys hashed): `AutoLoad`, `DisableAssetPreload`,
//!   `GetINILoadLastSave`, `IsDemoMode`, `PlayIntroMovies`,
//!   `StartWithResources`. Retail ships without that INI, so they keep
//!   their static values: all off except `PlayIntroMovies`, whose byte
//!   (`0x82B7C8FA`) starts at 1. `FinishedShell` (`0x83133EE1`) is set by
//!   the shell code (`0x82211C48`) and starts off.
//! - **Profile options** at the game's options block (`0x83134474`):
//!   `YAxisInverted` (+0x7D), `IsAutosaveEnabled` (+0x7F, written by
//!   `SetAutosaveEnabled`), `RumbleEnabled` (+0x81). `ForceNextAutosave`
//!   sets the byte at +0x5384 and answers nothing.
//!
//! `IsConfirmOnCircle` asks whether the button layout (`0x82B7C5F8`, 6 at
//! start) is 8. `HaveActiveProfile` is true when the profile manager's
//! "no profile" byte is clear; `IsGermanSKU` reads the SKU byte. Both
//! answer nothing when called with no argument (`0x827DED18` checks for
//! one), matching the scripts' habit of passing the player.
//! `SetAutosaveEnabled` needs a boolean; anything else answers nil.
//!
//! Invented: the starting profile options (autosave on, rumble on,
//! Y axis not inverted) — the 360 loads them from the player's profile,
//! and the defaults that profile code writes weren't traced. The port
//! always has a profile and is never the German SKU.

use mlua::{MultiValue, Value};

/// Button layout value `IsConfirmOnCircle` asks for.
const CONFIRM_ON_CIRCLE: u32 = 8;

/// The `Sys` flags the scripts can read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SysFlags {
    pub auto_load: bool,
    pub disable_asset_preload: bool,
    pub finished_shell: bool,
    pub ini_load_last_save: bool,
    pub demo_mode: bool,
    pub play_intro_movies: bool,
    pub start_with_resources: bool,
    pub y_axis_inverted: bool,
    pub autosave_enabled: bool,
    pub rumble_enabled: bool,
    /// Set by `Sys.ForceNextAutosave`.
    pub force_next_autosave: bool,
    /// The button layout (`0x82B7C5F8`).
    pub button_layout: u32,
    pub have_profile: bool,
    pub german_sku: bool,
}

impl Default for SysFlags {
    fn default() -> Self {
        Self {
            auto_load: false,
            disable_asset_preload: false,
            finished_shell: false,
            ini_load_last_save: false,
            demo_mode: false,
            play_intro_movies: true,
            start_with_resources: false,
            y_axis_inverted: false,
            autosave_enabled: true,
            rumble_enabled: true,
            force_next_autosave: false,
            button_layout: 6,
            have_profile: true,
            german_sku: false,
        }
    }
}

/// One boolean answer.
pub fn yes_no(b: bool) -> MultiValue {
    MultiValue::from_vec(vec![Value::Boolean(b)])
}

/// The getters, by Lua name: what each answers with no arguments needed.
pub fn getter(flags: &SysFlags, name: &str) -> Option<bool> {
    Some(match name {
        "IsFinalConfig" => true,
        "NoHud" => false,
        "AutoLoad" => flags.auto_load,
        "DisableAssetPreload" => flags.disable_asset_preload,
        "FinishedShell" => flags.finished_shell,
        "GetINILoadLastSave" => flags.ini_load_last_save,
        "IsDemoMode" => flags.demo_mode,
        "PlayIntroMovies" => flags.play_intro_movies,
        "StartWithResources" => flags.start_with_resources,
        "YAxisInverted" => flags.y_axis_inverted,
        "IsAutosaveEnabled" => flags.autosave_enabled,
        "RumbleEnabled" => flags.rumble_enabled,
        "IsConfirmOnCircle" => flags.button_layout == CONFIRM_ON_CIRCLE,
        _ => return None,
    })
}

/// The getters' Lua names.
pub const GETTERS: [&str; 13] = [
    "IsFinalConfig",
    "NoHud",
    "AutoLoad",
    "DisableAssetPreload",
    "FinishedShell",
    "GetINILoadLastSave",
    "IsDemoMode",
    "PlayIntroMovies",
    "StartWithResources",
    "YAxisInverted",
    "IsAutosaveEnabled",
    "RumbleEnabled",
    "IsConfirmOnCircle",
];

/// `Sys.HaveActiveProfile` / `Sys.IsGermanSKU`: the answer, or nothing
/// without an argument.
pub fn with_arg(args: &[Value], b: bool) -> MultiValue {
    if args.is_empty() {
        MultiValue::new()
    } else {
        yes_no(b)
    }
}

/// `Sys.SetAutosaveEnabled(on)`: nothing on success, nil when `on` isn't
/// a boolean.
pub fn set_autosave(flags: &mut SysFlags, args: &[Value]) -> MultiValue {
    match args.first() {
        Some(Value::Boolean(b)) => {
            flags.autosave_enabled = *b;
            MultiValue::new()
        }
        _ => MultiValue::from_vec(vec![Value::Nil]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retail_values() {
        let f = SysFlags::default();
        let on: Vec<&str> = GETTERS
            .iter()
            .copied()
            .filter(|n| getter(&f, n).unwrap())
            .collect();
        assert_eq!(
            on,
            [
                "IsFinalConfig",
                "PlayIntroMovies",
                "IsAutosaveEnabled",
                "RumbleEnabled"
            ]
        );
        assert_eq!(getter(&f, "Nope"), None);
    }

    #[test]
    fn confirm_on_circle_is_layout_eight() {
        let mut f = SysFlags::default();
        assert_eq!(getter(&f, "IsConfirmOnCircle"), Some(false));
        f.button_layout = 8;
        assert_eq!(getter(&f, "IsConfirmOnCircle"), Some(true));
    }

    #[test]
    fn set_autosave_needs_a_boolean() {
        let mut f = SysFlags::default();
        assert!(set_autosave(&mut f, &[Value::Boolean(false)]).is_empty());
        assert!(!f.autosave_enabled);
        let r = set_autosave(&mut f, &[Value::Integer(1)]);
        assert_eq!(r.len(), 1);
        assert!(matches!(r.into_iter().next(), Some(Value::Nil)));
        assert!(!f.autosave_enabled);
    }

    #[test]
    fn profile_questions_need_an_argument() {
        assert!(with_arg(&[], true).is_empty());
        assert_eq!(with_arg(&[Value::Integer(0)], true).len(), 1);
    }
}
