# Mod manager: design note (not built yet)

Status: a proposal for review. No code. `m2-modapi` today is an empty
crate (its doc line only), and nothing in the game reads mods yet.

## Goal

Players turn mods on and off, and order them, from the launcher's
**Mods** page (now "soon" in the sidebar). The game loads exactly the
enabled set, in that order, through `m2-modapi`. Mods carry only their
authors' own files. The project never ships, downloads or repacks game
assets.

## What a mod is

A folder under `mods\<id>\` beside the launcher and game, holding a
`mod.toml` and the mod's own files:

```toml
id = "better-jeep"            # folder-safe, unique
name = "Better jeep handling"
version = "1.2.0"
author = "..."
description = "One or two sentences."
game = ">=0.1"                # the port builds it works with
# What it changes; each kind is loaded by its own m2-modapi loader.
[[override]]
kind = "script"               # a Lua module replaced or added, by module name
module = "MrxVehicleTuning"
file = "scripts/MrxVehicleTuning.lua"
[[override]]
kind = "data"                 # template values (TOML patch on the template DB)
file = "data/jeep.toml"
[[override]]
kind = "texture"              # the mod's own picture, under an asset name
asset = "veh_jeep_body"
file = "textures/jeep.png"
```

The kinds are only the three above to begin with: scripts, data patches
and textures. Each is small, testable and maps to a loader the port
already has (the script library's modules, the template database, the
texture cache). Models, sounds and movies come later.

## The pieces

1. **`m2-modapi`** (library, no UI):
   - **Discovery:** parse and check each `mod.toml`. Refuse executables, absolute paths and `..`, set size caps, and require files that exist and match their kind.
   - **The load set:** the enabled ids and their order, from `mods.toml` (see 2). Where two mods override the same thing, the later one wins and the conflict is listed.
   - **A hash of the set,** for saves and co-op.
   - **Typed override tables** the game asks for (`script(module)`, `data_patches()`, `texture(name)`). The game owns how they apply.
   - Tests on synthetic mods only.
2. **Settings:** `mods.toml` beside the game's settings, holding the enabled ids in order. The launcher writes it; the game only reads it. A `--no-mods` switch starts clean.
3. **m2-alpha:**
   - loads the set at start (before the scripts and world), logs it, and applies the overrides through the loaders above;
   - writes the set's hash into the save record's own extra block, and warns on load if it differs;
   - in co-op, the host's hash must match the joiner's (refuse with a message otherwise).
4. **Launcher Mods page:**
   - the list with on/off and up/down;
   - each mod's name, version, author, description and the build range;
   - conflicts and problems shown on its row;
   - "Open mods folder", and "Install from .zip" (local file only: unpacked into `mods\<id>\` after the same checks);
   - a mod folder is removed only on the player's click, with a confirmation.

## Safety and limits

- Lua mods run in the game's script host with the same natives as the game's own scripts. That is powerful, and only safe for mods the player trusts; the page says so. A later step could deny some natives (file and network access are not natives today).
- There is no online catalogue or download in the first version, so nothing is fetched by the launcher for mods.
- Mods are off on the nightly autoplay runs (`--no-mods`), so mod state never makes a nightly result.

## Steps (each its own PR)

1. `m2-modapi` discovery, checks, order, conflicts and hash, with tests. No game change.
2. m2-alpha reads `mods.toml` and applies **data** patches only, behind `--mods` (logged), with a check flag.
3. Script overrides, with a check flag that loads a test mod's module.
4. Texture overrides.
5. The launcher's Mods page, reading and writing `mods.toml`.
6. Save hash and co-op match.

## Open questions for review

- Is `mods\` beside the game right, or `%LOCALAPPDATA%\m2-alpha\mods` (where photo mode's pictures go)?
- Should script overrides replace whole modules only, or also allow adding functions to a module?
- Should co-op refuse mismatched sets, or warn and play on?
- Is `game` (the build range) needed in version 1, or is the mod's own version enough?
