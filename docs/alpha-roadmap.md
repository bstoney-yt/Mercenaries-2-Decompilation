# Alpha roadmap

Status as of 2026-10-03 (after PR #21). This is a plan, not a spec: sizes are
rough, and each item becomes one PR.

## What alpha means

**Alpha is a native Rust executable (`m2-alpha`) that reads the player's own
install and lets them play the island with the systems the browser viewer
already has.** It is not the browser viewer, and it is not a mod loaded into
the original game.

Why this and not the alternatives:

- **Not a hook DLL inside the original exe.** The scaffold planned a
  `d3d9.dll` proxy (`m2-loader`, `m2-hook`), but the engine code is
  SecuROM-encrypted (`.text` 0x401000–0x6DE000) and defeating that is off
  the table. With no engine code to replace function by function, the hook
  path has nothing to hook. `m2-loader`/`m2-hook`/`m2-oracle` stay parked.
- **Not the browser viewer.** It proves the decoders and the gameplay ideas,
  but it is 1,900 lines of JavaScript on three.js from a CDN, needs a glTF
  export step, and cannot use `m2-render`. The project goal is a Rust engine
  behind the `RenderBackend` trait, so gameplay moves to Rust.

Everything gameplay-side stays clean-room and *invented* (tuning, AI, damage),
using the game's own data wherever it decodes.

### Exit criteria

1. `m2-alpha --install "<install dir>"` opens a window on Windows 10/11 and
   streams the island straight from `vz.wad` (no `dumps/` step).
2. On foot: walk/run/sprint with game clips, terrain and PHY2 collision.
3. Vehicles: enter/exit, drive cars/trucks, fly a helicopter.
4. People: soldiers at their posts, civilians; both react to the player.
5. Combat: rifle, RPG, grenades, explosions, vehicle wrecks, player death and
   respawn.
6. Audio: 3D effects, engines, ambience, music.
7. A HUD (health, ammo, minimap) and a pause menu (resume, settings, quit).
8. Save and load the player's position, health and inventory.
9. 60 fps at 1080p on a mid-range GPU in the town; no crash in a 30-minute
   session; `cargo test --workspace` and CI green.
10. Nothing from the game is in the repo or in release builds.

Missions are **not** an alpha criterion (see decision 3).

## Task list

Sizes: S ≈ one PR in a day, M ≈ a few days, L ≈ a week or more.
Source: **D** = decodable from the install, **I** = invented, **D+I** = game
data with invented behaviour.

| # | Task | Size | Depends on | Source | Files it owns |
|---|---|---|---|---|---|
| 1 | **wgpu backend** for `RenderBackend` (textured, skinned, instanced, alpha, sky/sea) | L | — | I | `m2-render/src/wgpu/` |
| 2 | **App shell**: `m2-alpha` binary, winit window, input map, fixed-step loop, config | M | — | I | new `crates/m2-alpha` |
| 3 | **Asset streaming**: load models/textures/clips by hash from `vz.wad` into GPU buffers on a worker thread, LRU by distance | M | 1 | D | `m2-engine/src/assets.rs` |
| 4 | **World**: terrain cells + tiles, placements, scatter, sea; port from `examples/gltf.rs` into a library | M | 3 | D | `m2-engine/src/world.rs` |
| 5 | **Animation runtime**: pose sampler (exists) + blending, root motion, skinning palette | M | 3 | D | `m2-engine/src/anim.rs` |
| 6 | **Physics**: PHY2 shapes into a physics library, character controller (0.5 m step, 50° slope) | M | 4 | D+I | `m2-engine/src/physics.rs` |
| 7 | **Audio**: PCM banks + IMA ADPCM streams into a mixer with 3D panning | M | 2 | D | `m2-engine/src/audio.rs` |
| 8 | **Player on foot** (port viewer Play mode) | M | 5, 6 | D+I | `m2-game/src/player.rs` |
| 9 | **Vehicles** (port drive/fly/enter/exit) | M | 8 | D+I | `m2-game/src/vehicle.rs` |
| 10 | **NPCs** (port spawners, civilians) | M | 8 | D+I | `m2-game/src/npc.rs` |
| 11 | **Combat** (port rifle, RPG, grenades, wrecks) | M | 9, 10 | D+I | `m2-game/src/combat.rs` |
| 12 | **HUD + menus** (egui or similar over wgpu; font from install if decodable, else bundled free font) | M | 1, 2 | I | `m2-alpha/src/ui/` |
| 13 | **Save/load** (own format, serde + TOML/bincode) | S | 8–11 | I | `m2-game/src/save.rs` |
| 14 | **Performance + stability pass**, crash log, frame-time HUD | M | all | — | cross-cutting |
| 15 | **Alpha release**: Windows build in CI, install-path check, README | S | 14 | — | `.github/`, `README.md` |

**Parallel lanes** (no shared files):

- Lane A: 1 → 3 → 4 → 6
- Lane B: 2 → 12
- Lane C: 7 (needs only the decoders and a stub app)
- Lane D: 5 (needs only the decoders and a test harness)

Lanes join at 8. Items 9 and 10 can then run in parallel; 11 joins them.
The viewer stays as the reference while porting, and is frozen (bug fixes
only) once item 11 lands.

## Known gaps

| Gap | Effect on alpha | Plan |
|---|---|---|
| Asset name hash unknown | Can't look up assets by name; town batch models fall back to render-triangle collision; game's parked cars missing | Keep keying by hash. Retry only with new input (see `wad-format` notes). Fallback collision is acceptable for alpha. |
| Per-building collision | Some walls are triangle soup, not shapes | Same as above; triangle-mesh colliders in the physics library. |
| Parked cars | Vehicles are placed by us | Keep invented placement near roads (`Road` layer component). |
| Soldier types, patrol routes | Every soldier looks and acts the same | Inspect the `Ai` layer component during item 10; otherwise invent. |
| Mission scripts, objectives | No missions | Out of alpha scope; investigate after alpha (decision 3). |
| UI / HUD / fonts | Game's Scaleform GFx menus are Flash files | Our own HUD for alpha; GFx is a separate project. |
| Save format | Game saves are unknown | Our own format; not compatible with the original. |
| Dialogue triggers | 12,988 voice lines unused | Owned by the voice-work thread; alpha just exposes an audio API for it. |
| Bink movies (`.bik`) | No cutscenes | Skip for alpha. |
| Sea level | -45 m, fitted by eye to the beaches (-35 m drowned them) | `[world] sea_level` in the settings file, or `--sea`. |

## Legal guardrails

- The repo and every release contain only our code. No game files, no
  extracted assets, no dumps; the binary reads the player's own install at
  run time.
- No DRM circumvention: we never decrypt, unpack or patch the SecuROM-protected
  exe, and the alpha does not need the exe at all.
- No engine code: behaviour is designed from observed data and general
  practice. Names from the exe's readable strings are fine; code is not.
- Third-party crates are permissively licensed (MIT/Apache/zlib/BSD only).
- README states that a legally owned copy is required.

## Decisions (taken 2026-10-03)

1. **Graphics API: wgpu.** D3D12 and Vulkan through one backend, matching
   the "modern APIs" goal, rather than a legacy D3D9 backend.
2. **Physics: Rapier.** Saves weeks on character, vehicle and debris
   physics; tuning stays invented either way.
3. **Missions: out of alpha.** Mission data isn't decoded; alpha doesn't
   wait on it.

## Progress

- Items 1 and 2: `m2-render::gpu::WgpuBackend` (textures, materials, sun
  and fog lighting, reversed depth, D3D9 winding; offscreen mode for tests)
  and the `m2-alpha` binary (winit window, settings file, rebindable keys,
  60 Hz fixed step, free-fly camera, `--screenshot`). Skinning and
  instancing move to items 5 and 4, where they are first needed.
- Item 3: `m2-engine::assets` (install discovery, models and textures by
  hash with a block cache), `m2-engine::model` (decode on any thread,
  upload to any backend) and `m2-engine::stream` (a loader thread, nearest
  first, uploads within a per-frame budget).
- Item 4 started: `m2-engine::world` indexes the island from its entity
  layers alone, and the window streams low-res tiles everywhere, detailed
  cells within 600 m, placed models within 1.2 km and a sea plane. Ground
  scatter (grass, shrubs, rubble) streams within 150 m as instanced draws
  (`DrawCall::instances`). Near the camera the cells' shared detail texture (material slot 1) is tiled over the colour map every 6 m (an invented scale). Still to come: the tiled layer textures (`tt_sand01`...), whose blending by vertex colour is not confirmed.
- Item 5: `m2-engine::anim` binds clips to any skeleton by bone name, blends
  gaits, plays one-shot clips (aim, death) and builds skinning palettes;
  the wgpu backend skins on the GPU.
- Item 6: `m2-engine::physics` (Rapier) loads PHY2 shapes and the
  triangle fallback, with a kinematic character controller (0.5 m step,
  50° slope) and raycast vehicles.
- Item 7: `m2-engine` mixes the PCM banks and the streamed music and
  ambience; `m2-alpha::sound` plays ambience, music and fight sounds in 3D
  (voice chatter is not wired in yet).
- Items 8 to 11, in `m2-alpha` for now: Jennifer on foot, enter, drive and
  get out of the parked vehicles, soldiers at their spawns (patrols where
  the layer names a route) and wandering civilians, rifle, RPG, grenades,
  wrecks. Handling, damage and AI timings are invented.
- Item 12: egui HUD (health, weapon, speed, prompts, crosshair) and an Esc
  pause menu (field of view, mouse, vsync, controls, save, load, quit).
- Item 13: F5/F9 quick save and load (`m2-alpha-save.toml`: player,
  vehicles, soldiers killed).
- Item 14 started: the model catalogue search (12-14 s) is kept between
  runs in `m2-alpha-catalogue.toml`.
- Item 15 started: the game is found in the EA app's Program Files folder
  without `--install`, and every CI run keeps a release `m2-alpha.exe` as
  an artifact.
