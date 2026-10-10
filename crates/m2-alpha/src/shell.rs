//! The front end as the game opens it: the shell's own scripts and Flash
//! menus, run on a [`GameHud`](crate::gamehud::GameHud) thread
//! ([`GameHud::start_shell`](crate::gamehud::GameHud::start_shell)).
//!
//! After the boot movies the game's `MrxGuiShellBootstrap.Init` loads the
//! loading screen's layout and then the movie layouts (the cinematics' and
//! the fades'); `EnterShell` loads `MrxGuiShellLayout`, whose widget's
//! initialisation (`MrxGuiShell.HandleInitializationEvent`) makes a movie
//! widget on the right (the backdrop videos) and a full-screen Flash widget
//! playing `shell.gfx`, and hooks the movie's commands; once loaded
//! (`ShellScreenLoaded`) the bootstrap asks for the `Shell` game state, and
//! the engine's `GuiGameStateChange` (`Shell`, `Enter`) opens it.
//!
//! The movie's commands do the rest: `startMovie`/`stopMovie` with a
//! backdrop's name play it in the movie widget, and starting a new game
//! (`LTIStartNewGame` with the character on the PC; `newGame` elsewhere)
//! sets the character (`MrxGuiShellBootstrap.SetSelectedCharacter`) and
//! calls `Sys.StartSingleplayer`, which here notes the character for the
//! game ([`take_new_game`]). The movie's own `newGame` carries no
//! character: the engine starts the game as the profile's
//! ([`Native::character`], [`start_new_game`]).
//!
//! The menu movie itself waits on the engine: its first frame stops until
//! told the platform, and its press-start screen (`pressStartEnabled`)
//! until a key starts the shell. What the engine tells it is what the
//! movie's own test path (taken when its `testingVAR` is set) tells it:
//! `setPlatform`, `firstTimeLoad(true)`, `setRegion("US")` and
//! `setTerritory("EN")`, with the platform the PC's (`PC`, which its text
//! tables tell apart); then, once press-start is up, Enter calls
//! `startShell` (see [`Native`]).
//!
//! `LTILibName` is the PC version's own library behind its options
//! screens (video modes, key bindings, the profile): the engine's, not the
//! scripts'. Its functions answer nothing here (each noted once), but
//! `FirstRun`, which says this profile has run before (no wait before the
//! legal screen can be skipped).
//!
//! # Provenance
//!
//! Read from the game's shell scripts (`MrxGuiShellBootstrap`,
//! `MrxGuiShell`) in a legally installed copy. Nothing from the
//! executable.

use m2_game::gui_runtime;
use m2_game::mission::Mission;
use mlua::Lua;

/// Sets the shell's world up and opens it.
const OPEN: &str = r#"
local me = Sys.StringToGuid("0x1")
for _, f in ipairs({ "GetLocalPlayer", "GetPrimaryPlayer" }) do
    rawset(Player, f, function() return me end)
end
rawset(Player, "GetAllPlayers", function() return { me } end)
local noted = {}
LTILibName = setmetatable({
    FirstRun = function() return false end,
}, {
    __index = function(_, k)
        return function()
            if not noted[k] then
                noted[k] = true
                __m2_note("LTILibName." .. tostring(k))
            end
        end
    end,
})
rawset(Sys, "StartSingleplayer", function()
    local who = MrxGuiShellBootstrap.GetSelectedCharacter
        and MrxGuiShellBootstrap.GetSelectedCharacter()
    __m2_new_game = tostring(who or "")
end)
if rawget(Sys, "GetMasterScriptName") == nil then
    rawset(Sys, "GetMasterScriptName", function() return "" end)
end
import("MrxGuiShellBootstrap")
MrxGuiShellBootstrap.Init()
MrxGuiShellBootstrap.EnterShell()
"#;

/// Opens the shell in `mission`'s Lua state (a mission made from
/// `shell.wad`, not started).
pub fn open(mission: &mut Mission) -> Result<(), String> {
    mission.run(OPEN, "m2 shell").map_err(|e| e.to_string())?;
    // The layouts load: their callbacks run (`ShellScreenLoaded`, which
    // asks for the `Shell` game state: see [`game_states`]).
    gui_runtime::tick(mission.lua(), 0.0);
    gui_runtime::tick(mission.lua(), 0.0);
    Ok(())
}

/// The game states the scripts asked for (`Sys.RequestGameState`) among
/// `commands`, entered as the engine enters them: the GUI is told the
/// state left (`GuiGameStateChange` with `Exit`) and the one entered
/// (`Enter`). `state` is the state the shell is in. Returns lines for the
/// log.
pub fn game_states(
    lua: &Lua,
    commands: &[m2_game::api::Command],
    state: &mut Option<String>,
) -> Vec<String> {
    let mut log = Vec::new();
    for c in commands {
        let m2_game::api::Command::GameState(next) = c else {
            continue;
        };
        let mut tell = |name: &str, phase: &str| match gui_runtime::fire(
            lua,
            "GuiGameStateChange",
            (name.to_owned(), phase),
        ) {
            Ok(n) => log.push(format!("shell: game state {name} {phase} ({n} handlers)")),
            Err(e) => log.push(format!("shell: game state {name} {phase}: {e}")),
        };
        if let Some(old) = state.take() {
            tell(&old, "Exit");
        }
        tell(next, "Enter");
        *state = Some(next.clone());
    }
    log
}

/// The character a new game was started as since the last call
/// (`Chris`, `Jen` or `mattias`), if one was.
pub fn take_new_game(lua: &Lua) -> Option<String> {
    let g = lua.globals();
    let who = g
        .raw_get::<Option<String>>("__m2_new_game")
        .ok()
        .flatten()?;
    let _ = g.raw_set("__m2_new_game", mlua::Value::Nil);
    Some(who)
}

/// The hero the shell's character name stands for (`Chris`, `Jen`,
/// `mattias`, any case), if it names one; one that does not is logged.
pub fn hero(character: &str) -> Option<crate::hero::Hero> {
    let hero = crate::hero::Hero::parse(character);
    if hero.is_none() {
        eprintln!("front end: the character {character:?} names no hero");
    }
    hero
}

/// The engine's side of the menu movie (`shell.gfx`): what it calls in
/// the movie, and when.
#[derive(Debug, Default)]
pub struct Native {
    /// The movie has been told the platform.
    set_up: bool,
    /// The press-start screen is up (`pressStartEnabled`), waiting for a
    /// key.
    press_start: bool,
    /// Calls to make at the next frame.
    pending: Vec<Call>,
    /// The save operation waiting on the profile's save.
    saving: Option<&'static str>,
    /// The profile's character, as the movie last set it
    /// (`onlineSetParameters` with `profileCharacter`): see
    /// [`Native::character`].
    character: Option<&'static str>,
}

/// The menu movie's name.
pub const MOVIE: &str = "shell";

/// The engine's calls into the movie: the function's name and its
/// arguments.
pub type Call = (&'static str, Vec<m2_assets::flash::avm1::Value>);

/// `onlineLTIIsConnected`'s page for a new game (the main menu's New
/// Game sets `ProfilePage` 2 and asks with it).
const NEW_GAME_PAGE: &str = "2";

/// `saveCheck`'s operation for a new game (accepting a character).
const NEW_GAME_OPERATION: &str = "newGame";

impl Native {
    /// What to call in the movie now: set up as it starts (once), then
    /// the answers to what it asked.
    pub fn calls(&mut self) -> Vec<Call> {
        use m2_assets::flash::avm1::Value;
        let mut calls = Vec::new();
        if !std::mem::replace(&mut self.set_up, true) {
            calls.extend([
                ("setPlatform", vec![Value::Str("PC".into())]),
                ("firstTimeLoad", vec![Value::Bool(true)]),
                ("setRegion", vec![Value::Str("US".into())]),
                ("setTerritory", vec![Value::Str("EN".into())]),
            ]);
        }
        calls.append(&mut self.pending);
        calls
    }

    /// An `fscommand` no script handles: whether it is the engine's.
    ///
    /// `onlineLTIIsConnected` asks the PC's library whether the player
    /// may go on to the page asked for. For a new game (single player)
    /// the answer is `LTIgotoGame`, whose page 2 goes on to the
    /// characters (`newGameSelect`; its trace says "new game - success").
    /// The other pages (continue and load: saves; join: online) are not
    /// answered.
    pub fn command(&mut self, command: &str, arg: &str) -> bool {
        use m2_assets::flash::avm1::Value;
        match command {
            "pressStartEnabled" => {
                self.press_start = arg != "false";
                true
            }
            "onlineLTIIsConnected" => {
                if arg == NEW_GAME_PAGE {
                    self.pending.push(("LTIgotoGame", Vec::new()));
                }
                true
            }
            // `saveCheck(perform, what)` asks the engine to get the saves
            // ready (`preoperation` with "perform,what") and waits for
            // `preopComplete(true, what)`, which saves the profile
            // (`onlineSaveProfile`) and waits again, for
            // `saveCompleteContinue(what)`: what the movie's own test path
            // calls straight away at each step. A new game's goes on to
            // start it; the others need saves, which there are none of yet.
            "preoperation" => {
                if arg.split(',').nth(1) == Some(NEW_GAME_OPERATION) {
                    self.pending.push((
                        "preopComplete",
                        vec![Value::Bool(true), Value::Str(NEW_GAME_OPERATION.into())],
                    ));
                    self.saving = Some(NEW_GAME_OPERATION);
                }
                true
            }
            "onlineSaveProfile" => {
                if let Some(what) = self.saving.take() {
                    self.pending
                        .push(("saveCompleteContinue", vec![Value::Str(what.into())]));
                }
                true
            }
            // The movie's settings for the profile (`outputSettings`, one
            // "name, value" each), saved by the engine: the character is
            // the one accepted on the character screen.
            "onlineSetParameters" => {
                if let Some(("profileCharacter", n)) = arg.split_once(", ") {
                    self.character = profile_character(n.trim()).or(self.character);
                }
                true
            }
            _ => false,
        }
    }

    /// The character the profile was given (`chris`, `jennifer` or
    /// `mattias`, as the shell's scripts name them), once the movie has
    /// set it.
    ///
    /// The movie starts the game with `fscommand("newGame", 1)` from its
    /// `_newgameStart` frame: the argument is a constant, not the
    /// character. On the PC the character is the profile's
    /// (`profileCharacter`), which the engine hands the scripts as
    /// `sCharacter` for their `LTIStartNewGame` handler
    /// (`MrxGuiShell._LTIStartNewGame`, which reads that global, not its
    /// argument).
    pub fn character(&self) -> Option<&'static str> {
        self.character
    }

    /// A key (Flash's code) going down: what to call for it.
    pub fn key(&mut self, code: i32) -> Vec<Call> {
        if self.press_start && code == 13 {
            self.press_start = false;
            return vec![("startShell", Vec::new())];
        }
        Vec::new()
    }
}

/// The scripts' name for the movie's `profileCharacterVar`: 1, 2 and 3 are
/// Mattias, Chris and Jennifer (its character buttons' order, which
/// `newGameTransition` and the saves' pages read the same way).
fn profile_character(n: &str) -> Option<&'static str> {
    match n {
        "1" => Some("mattias"),
        "2" => Some("chris"),
        "3" => Some("jennifer"),
        _ => None,
    }
}

/// Starts the new game the menu movie asked for (`fscommand("newGame", _)`
/// from widget `widget`) as the PC does, as `character` (see
/// [`Native::character`]): the scripts' `sCharacter` is set and their
/// `LTIStartNewGame` handler runs if the widget has one; otherwise their
/// `newGame` handler (`_NewGameFlashCallback`, which takes the character
/// as its argument) is given it. Returns whether a handler ran.
pub fn start_new_game(lua: &Lua, widget: u32, character: &str) -> mlua::Result<bool> {
    lua.globals().set("sCharacter", character)?;
    if gui_runtime::flash_command(lua, widget, "LTIStartNewGame", character)? {
        return Ok(true);
    }
    gui_runtime::flash_command(lua, widget, "newGame", character)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(calls: Vec<Call>) -> Vec<&'static str> {
        calls.into_iter().map(|c| c.0).collect()
    }

    #[test]
    fn the_movie_is_set_up_once_and_enter_starts_the_shell_from_press_start() {
        let mut n = Native::default();
        assert_eq!(
            names(n.calls()),
            ["setPlatform", "firstTimeLoad", "setRegion", "setTerritory"]
        );
        assert!(n.calls().is_empty());
        assert!(n.key(13).is_empty());
        assert!(n.command("pressStartEnabled", "true"));
        assert!(n.key(32).is_empty());
        assert_eq!(names(n.key(13)), ["startShell"]);
        assert!(n.key(13).is_empty());
        assert!(!n.command("startMovie", "x"));
    }

    #[test]
    fn a_new_game_goes_on_to_the_characters() {
        let mut n = Native::default();
        n.calls();
        assert!(n.command("onlineLTIIsConnected", "3"));
        assert!(n.calls().is_empty());
        assert!(n.command("onlineLTIIsConnected", "2"));
        assert_eq!(names(n.calls()), ["LTIgotoGame"]);
        assert!(n.command("preoperation", "load,"));
        assert!(n.calls().is_empty());
        assert!(n.command("preoperation", "save,newGame"));
        assert_eq!(names(n.calls()), ["preopComplete"]);
        assert!(n.command("onlineSaveProfile", "true"));
        assert_eq!(names(n.calls()), ["saveCompleteContinue"]);
        assert!(n.command("onlineSaveProfile", "true"));
        assert!(n.calls().is_empty());
    }

    #[test]
    fn only_the_heroes_names_pick_a_hero() {
        use crate::hero::Hero;
        assert_eq!(hero("Chris"), Some(Hero::Chris));
        assert_eq!(hero("Jen"), Some(Hero::Jennifer));
        assert_eq!(hero("jennifer"), Some(Hero::Jennifer));
        assert_eq!(hero("mattias"), Some(Hero::Mattias));
        // What a missing or stubbed `GetSelectedCharacter` leaves.
        assert_eq!(hero(""), None);
        assert_eq!(hero("table: 0x0123"), None);
        assert_eq!(hero("1"), None);
    }

    #[test]
    fn the_profile_keeps_the_accepted_character() {
        let mut n = Native::default();
        assert_eq!(n.character(), None);
        assert!(n.command("onlineSetParameters", "serverOnline, true"));
        assert_eq!(n.character(), None);
        assert!(n.command("onlineSetParameters", "profileCharacter, 2"));
        assert_eq!(n.character(), Some("chris"));
        assert!(n.command("onlineSetParameters", "profileCharacter, undefined"));
        assert_eq!(n.character(), Some("chris"));
        assert!(n.command("onlineSetParameters", "profileCharacter, 3"));
        assert_eq!(n.character(), Some("jennifer"));
        assert!(n.command("onlineSetParameters", "profileCharacter, 1"));
        assert_eq!(n.character(), Some("mattias"));
    }
}
