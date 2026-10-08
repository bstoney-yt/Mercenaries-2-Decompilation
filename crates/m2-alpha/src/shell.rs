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
//! game ([`take_new_game`]).
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

/// The hero the shell's character name stands for.
pub fn hero(character: &str) -> crate::hero::Hero {
    use crate::hero::Hero;
    match character.to_ascii_lowercase().as_str() {
        "chris" => Hero::Chris,
        "jen" | "jennifer" => Hero::Jennifer,
        _ => Hero::Mattias,
    }
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
            // `saveCheck(kind, operation)` asks the engine to get the
            // saves ready (`preoperation` with "kind,operation") and waits
            // for `preopComplete(operation, true)`: what the movie's own
            // test path calls straight away. A new game's goes on to start
            // it; the others need saves, which there are none of yet.
            "preoperation" => {
                if arg.split(',').nth(1) == Some(NEW_GAME_OPERATION) {
                    use m2_assets::flash::avm1::Value;
                    self.pending.push((
                        "preopComplete",
                        vec![Value::Str(NEW_GAME_OPERATION.into()), Value::Bool(true)],
                    ));
                }
                true
            }
            _ => false,
        }
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
    }
}
