# Missions plan

How missions get into m2-alpha. Everything here is read from the player's
own install at run time; no script, layer or string from the game is
committed or shipped.

## What the game has (from `wad -- scripts`, 2026-10-04)

- **Scripts.** vz.wad holds 645 compiled Lua 5.1 scripts (asset type
  42498680, little-endian). shell.wad holds 28 UI scripts, and English.wad
  and Loading.wad hold one each. Every one parses with the `m2-assets::lua`
  reader.
- **Where the logic is.** The `<faction><nnn>_job` / `_con` blocks (222 of
  them) hold only a briefing sequence (`sParticipant1`, `sParticipant2`,
  `tSequence`). The mission logic is in `scripts_vz` and `resident`: task
  types (`MrxTaskContract`, `MrxTaskObjective`, `MrxTaskJobDestroySet`,
  outposts), hijacks (29 vehicle-specific scripts plus the `tankhijack`
  quick-time events), support (airstrikes, designators), the PMC HQ and
  shop, and the GUI.
- **Modules.** Scripts load each other with `import` (257 scripts) and
  `inherit` (171). The import names are module names (`MrxUtil`,
  `MrxLayerManager`, and so on), while assets are keyed by the unsolved name
  hash. Imports are therefore resolved by matching each module's members
  used elsewhere (`MrxUtil.GetRandomTableElement`) against the globals each
  script defines.
- **Per-mission world state.** This lives in the `vz_state_*` entity
  layers, which the scripts add and remove through `MrxLayerManager.Add`
  and `MrxLayerManager.Remove`. Our layer parser already reads them.

## The engine API the scripts call

Counted across the 645 scripts. The full lists are on the dev PC in
`C:\Claude\scratch\missions\scripts-vz.txt`.

| Library | Most-used calls | Scripts |
|---|---|---|
| Event | Create, TimerRelative, Delete, ObjectHibernation, CreatePersistent, ObjectDeath, ObjectProximity, ObjectInSeat, ScriptEvent, Boundary, Post, ObjectHealth, ContextAction | 219 |
| Debug | Printf | 186 |
| Object | GetPosition, Remove, IsPlayerControlled, IsAlive, HasLabel, GetHealth, GetYaw/SetYaw, Kill, SetPosition, Attach/Detach, SetInvincible | 191 |
| Pg | GetGuidByName, Spawn, LoadAsset/UnloadAsset, FindPointFromCamera, Add/RemoveContextAction | 167 |
| Player | GetLocalCharacter, GetAnyCharacter, GetCharacter, GetAllPlayers, GetPrimary/SecondaryCharacter, GetCamera | 157 |
| Net | IsServer, IsClient, SendCustomEvent, IsActive | 86 |
| Vehicle | GetDriver, GetFromRider, Exit, GetRiders, Usable | 85 |
| Ai | Goal, Role, GetRelation | 67 |
| Hud | ObjectiveTray, Radar | 57 |
| Sound / music | CueSound, StopSound, MrxMusic.PlaySpecialMusic | 49 |
| Graphics, Airstrike, Human, Marker, Pda, Controller, Sys, String, Math | | 6 to 45 each |

The rest are the scripts' own `Mrx*` and `Wif*` modules and the standard
Lua library.

## Plan

1. **Run the scripts.** Load the game's bytecode into a stock Lua 5.1 VM
   (`mlua` with vendored Lua 5.1, MIT licence), in a new `m2-game::script`
   module. If the game's chunk sizes differ from the host's, `m2-assets::lua`
   re-emits the chunk in the host layout first. Resolve `import` and
   `inherit` from the module/member match above. Every engine library
   starts as a stub that logs its calls. The test is that `resident` and
   `scripts_vz` load and initialise without errors.
2. **Events and objects.** Implement the Event system: timers, proximity,
   death, seat, health, boundary, and script events posted between scripts.
   Map Object, Player, Vehicle and Pg guids to m2-alpha's people, vehicles
   and layer entities. `Pg.GetGuidByName` uses the entity names in the
   layers. `Net` reports a single-player server.
3. **Mission state.** `MrxLayerManager.Add` and `Remove` stream
   `vz_state_*` layers in and out of the world. The objective tray, radar
   blips and markers are drawn by our own HUD, not the game's Flash GUI.
   Briefing VO plays through the existing voice tables.
4. **First playable mission.** Pick the first contract the save defaults
   point at (`PmcCon001`), or a vehicle hijack as a smaller first target.
   Drive it start to finish headless with screenshots, then widen to the
   other contracts.

Out of scope for now: the Scaleform/Flash menus (`MrxGui*`, `SetSwfFile`),
co-op networking, and achievements. These get stubs that do nothing.
