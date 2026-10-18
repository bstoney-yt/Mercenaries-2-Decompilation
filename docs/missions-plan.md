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
  `MrxLayerManager`, and so on), and scripts are keyed by
  `m2_assets::name_hash` of that name (FNV-1a of the name with bit 5 set in
  each byte and a `*` appended; `MrxTask` is `d302a66f`). Before the hash was known,
  imports were resolved by matching each module's members used elsewhere
  (`MrxUtil.GetRandomTableElement`) against the globals each script
  defines; that match remains the fallback for a name no script's hash
  matches.
- **Per-mission world state.** This lives in the `vz_state_*` entity
  layers, which the scripts add and remove through `MrxLayerManager.Add`
  and `MrxLayerManager.Remove`. Our layer parser already reads them.

## The engine API the scripts call

Counted across the 645 scripts. The full lists are kept locally, outside the
repository.

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

## Objectives on screen (traced 2026-10-04, VzaCon001)

The scripts tell the HUD about objectives through these calls; the alpha
draws its own HUD from them. Text is never literal: every visible string is
a bracketed id such as `[VzaCon001.Objectives.001]`, joined with others and
inline icon tags (`[objt][objdeliver]`), with progress appended as
` (0/4)`. The ids are keys of the string tables (type `39e5e978`, the
English one named `english` in `vz.wad` and `shell.wad`, 18,299 strings)
through `m2_assets::name_hash`: `objective.Objectivecompleted` is
`923171d8`, "Objective completed:". `m2_game::hud::render` puts the
strings in; `m2-alpha --mission` shows them.

| Call | Arguments | Removed by |
|---|---|---|
| `Net.SendEvent_AddPDAMission` | mission index (VzaCon001 is 36), list of {text id, icon tag}, repeatable | re-sent whole when the list changes |
| `Net.SendEvent_AddPdaObjective` | name, target guid, text id, PDA icon index, mission index, state (0-2), sort order | `RemovePdaObjective(name)` |
| `Net.SendEvent_ObjectiveMessage` | inline icon index, `"add"` or `"cpl"`, text id, player guid or nil | (a message) |
| `Net.SendEvent_AddRadarObjective` | name, x, y, z offset, r, g, b, width, height, radar icon index, target guid, sticky, rotate, oriented, sort order | `RemoveRadarObjective(name)` |
| `Marker.AddBlip` | target guid, icon (`HUD_objective_deliverable`, `_action`, `_destroy`), size, r, g, b, a, height offset, near (5 m), far (175 m), size, mission id, 2D only; returns a handle | `Marker.Remove(handle)` |
| `Net.SendEvent_AddMarkerObjective` | target guid, marker handle, r, g, b, height offset, world icon index, ..., near, far, mission id hash | `RemoveMarkerObjective(handle)` |

`name` is the target guid as a string (`"734801"`), or `<mission>_PreMission`
for the blip that starts a mission. `Net.SendCustomEvent(module, event,
data, bool)` is script-to-script, not HUD.

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
