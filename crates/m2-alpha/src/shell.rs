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
