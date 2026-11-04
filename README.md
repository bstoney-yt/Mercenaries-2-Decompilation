<div align="center">

# Mercenaries 2: World in Flames, rebuilt in Rust

**A faithful, independently written native Rust recreation of Pandemic's 2008 open-world game.**
It reads your own installed copy of the PC game and brings none of the game's files.

[![CI](https://github.com/bstoney-yt/Mercenaries-2-Decompilation/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/bstoney-yt/Mercenaries-2-Decompilation/actions/workflows/ci.yml)
![Status: the Beta in progress](https://img.shields.io/badge/status-the%20Beta%20in%20progress-orange)
![Platform: Windows](https://img.shields.io/badge/platform-Windows-0078D6)
![Language: Rust](https://img.shields.io/badge/language-Rust-B7410E)
![Renderer: wgpu (D3D12 / Vulkan)](https://img.shields.io/badge/renderer-wgpu%20(D3D12%20%2F%20Vulkan)-5C2D91)
![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)

[What it is](#what-it-is) ·
[Where it stands](#where-it-stands) ·
[How we know what the game does](#how-we-know-what-the-game-does) ·
[Play it](#play-it) ·
[Build it](#build-it) ·
[Workspace](#workspace) ·
[Contributing](#contributing) ·
[Legal](#legal-disclaimer--anti-piracy-notice)

</div>

---

## What it is

Mercenaries 2 is not being emulated or patched here. Every system is rewritten from
scratch in Rust and runs on its own engine:

- 🎮 **`m2-alpha`** is the game itself: the island of Venezuela, the story, the
  contracts, the factions, vehicles, weapons, support and the HUD.
- 🖼️ **`m2-render`** draws it through [wgpu](https://wgpu.rs) (Direct3D 12 or Vulkan).
- 📦 **`m2-assets`** parses `vz.wad`, `shell.wad`, `English.wad` and `Loading.wad`
  from your own install at runtime: models, textures, terrain, templates, Lua
  scripts, Flash menus, sounds and movies.
- 🧩 **`m2-modapi`** holds data-driven definitions and hooks for mods.

The goal is the **actual game**, not a look-alike. Where the original behavior can
be established, we reproduce it; where it can't yet, the code says so.

## Where it stands

The current milestone is **the Beta**: single-player, Windows-only, and the whole
story playable from start to finish without skips.

| Area | State |
|---|---|
| 🌎 World | The full island streamed from your install: terrain, buildings, props, scatter, roads, water, day and night |
| 📜 Story and contracts | The game's own Lua mission scripts drive the story, contracts, jobs and faction moods; an autoplay play-through of the story is under way |
| 🖥️ Front end, HUD and menus | The game's own Flash movies: title screen, pause boxes, PDA and map, HUD, contract terms and the contacts' shops |
| 🚶 On foot | Movement, jumping, swimming, cover, aiming (in the crouched stance) and melee from the 360 code's human physics and locomotion |
| 🚙 Vehicles | Cars, tanks, bikes, boats and helicopters with collision from the game's own physics shapes; boat helm, drag and water springs, and helicopter controls, turbulence and AI pilots, all from the 360 code |
| 🪖 AI | The 360 goal planner and its task table; cover, line of fire, close-quarters fights, patrol boats, and turret gunners with the game's aim cones |
| 💥 Effects | Particles, explosions, forces and lights from the effect templates, run by the 360 particle step |
| 🔊 Sound and movies | The game's sound banks, music, voices and cut-scenes |
| 🛠️ Extras | The scripts' own developer cheat menu, quick save and load, Xbox controller support, and an Android build ([`android/`](android)) |
| 👥 Co-op | Online co-op works for heroes moving together; the shared world is paused until after the Beta |

The prioritized plan is in [docs/BETA_ROADMAP.md](docs/BETA_ROADMAP.md), with
readiness in [docs/BETA_READINESS.md](docs/BETA_READINESS.md) and the test plan in
[docs/BETA_TEST_PLAN.md](docs/BETA_TEST_PLAN.md). Known differences from the
original are tracked in [docs/fidelity/GAPS.md](docs/fidelity/GAPS.md).

## How we know what the game does

Behavior is established from evidence, in this order:

1. **The Xbox 360 program**, studied in Ghidra, is the source of truth for engine
   behavior: how values are loaded, transformed and applied.
2. **The game's data** in your install holds what was authored: templates, tuning
   tables, scripts and models.
3. **Footage of the original** confirms what can be seen and heard.

Every module records where its behavior came from:

| Tag | Meaning |
|---|---|
| **From the 360 code** | Established by tracing the original program |
| **From samples** | Read or inferred from the game's data |
| **From footage** | Observed in recordings of the original |
| **Invented** | Chosen on purpose where evidence doesn't yet establish the behavior, and listed as such |

Invented behavior is removed as soon as the code can establish the real thing. The
evidence levels and how each step is proven are explained in
[docs/fidelity/README.md](docs/fidelity/README.md).

## Play it

You need your own installed copy of *Mercenaries 2: World in Flames* for PC.

- **Download:** CI builds `m2-alpha.exe` (32-bit Windows) on every push to `main`
  and keeps it as a downloadable artifact on the
  [Actions](https://github.com/bstoney-yt/Mercenaries-2-Decompilation/actions/workflows/ci.yml)
  page. An early preview is on the
  [Releases](https://github.com/bstoney-yt/Mercenaries-2-Decompilation/releases) page.
- **Run it** with no options for the whole game: boot movies, the title screen, the
  first contract and on into free roam. The EA app install is found on its own;
  otherwise pass `--install "<install dir>"`.
- **Movies** are decoded with `ffmpeg` (set `M2_FFMPEG`, put it beside the exe, or
  have it on the `PATH`). Without it they are skipped.

| Key | Does |
|---|---|
| Esc | Pause menu: settings, controls, save, load, quit |
| Enter / Space / Esc | Skip a movie |
| Xbox controller | The game's own pad layout and button pictures |

`m2-alpha --help` lists every option and control, and `--write-config` writes a
settings file (install path, window, keys).

## Build it

A Cargo workspace using the toolchain pinned in [`rust-toolchain.toml`](rust-toolchain.toml).
The shipping target is `i686-pc-windows-msvc`; everything that doesn't touch Windows
also builds and tests on a 64-bit host.

```sh
cargo run --release -p m2-alpha                                    # the whole game
cargo run --release -p m2-alpha -- --install "<install dir>"       # point at your install
cargo run --release -p m2-alpha -- --no-boot --dev                 # skip the boot movies, developer keys on
cargo test --workspace
cargo clippy --workspace --target i686-pc-windows-msvc -- -D warnings
cargo fmt --check
cargo xtask symbols                                                # validate the symbol tables
```

<details>
<summary><b>Developer launches and asset tools</b></summary>

```sh
# Game launches for testing
cargo run --release -p m2-alpha -- --spawn 0,0                     # free roam from game x, z
cargo run --release -p m2-alpha -- --mission --skip-to PirCon001   # skip the story to a contract
cargo run --release -p m2-alpha -- --model 15d89832 --model <hash> # models side by side on a grid

# Asset tools (all read your own install; outputs stay on your machine)
cargo run -p m2-assets --example wad -- list "<install dir>\data\Loading.wad"
cargo run --release -p m2-assets --example wad -- world "<install dir>\data\vz.wad"   # check every terrain cell, layer, collision shape and sound bank
cargo run --release -p m2-assets --example gltf -- rigs "<install dir>\data\vz.wad"
cargo run --release -p m2-assets --example gltf -- export "<install dir>\data\vz.wad" 15d89832 dumps/jen.glb --filter jennifer
cargo run --release -p m2-assets --example gltf -- world "<install dir>\data\vz.wad" dumps/world.glb --detail 450,-1950 --radius 500
cargo run --release -p m2-assets --example gltf -- view dumps/jen.glb --world dumps/world.glb   # open http://127.0.0.1:8765
```

</details>

## Workspace

| Crate | Purpose |
|---|---|
| [`m2-alpha`](crates/m2-alpha) | The native game executable |
| [`m2-render`](crates/m2-render) | Rendering behind `RenderBackend`, with a wgpu backend (Direct3D 12 / Vulkan) |
| [`m2-assets`](crates/m2-assets) | Parsers for the files in your own install |
| [`m2-modapi`](crates/m2-modapi) | Data-driven mod definitions and hooks |
| [`m2-engine`](crates/m2-engine) | Engine runtime: assets from the install, models on the GPU, and the systems built on them |
| [`m2-game`](crates/m2-game) | Gameplay logic and the Lua script host |
| [`m2-net`](crates/m2-net) | Online co-op: wire protocol, invites and the peer-to-peer session |
| [`m2-layout`](crates/m2-layout) | `Ptr32<T>`, addresses and `assert_layout!` for binary-compatible structures |
| [`m2-math`](crates/m2-math) | D3DX-layout vectors and matrices, converted to `glam` |
| [`m2-mem`](crates/m2-mem) | Ownership-based replacements for the original allocators and pools |
| [`m2-sys`](crates/m2-sys) | All raw FFI, each `unsafe` block with its `// SAFETY:` note |
| [`m2-hook`](crates/m2-hook) | Symbol tables and the per-function original/Rust switch |
| [`m2-oracle`](crates/m2-oracle) | Differential tests against original functions (opt-in) |
| [`m2-runsum`](crates/m2-runsum) | Summarizes an autoplay run from its log |
| [`m2-loader`](crates/m2-loader) | The original in-process DLL approach (parked) |

[`symbols/`](symbols) tracks every identified function: name, address, signature and
translation status (see [symbols/README.md](symbols/README.md)).

## Contributing

- [docs/conventions.md](docs/conventions.md) is binding: `#[repr(C)]` for binary
  layouts, unknown fields named by offset, raw FFI only in `m2-sys`.
- Each change targets one measurable result, measured before and after, with tests
  that pin the established behavior and a provenance tag on what it implements.
- Commit original Rust code and symbol metadata only. **Never** commit game files,
  extracted assets, executable bytes, decompiler databases or raw decompiler output.
- Never copy code, shaders or assets from other projects.

## Legal Disclaimer & Anti-Piracy Notice

* **Educational & Research Purpose:** This project is an open-source reverse-engineering effort created solely for educational, preservation, and interoperability purposes.
* **No Copyrighted Assets:** This repository does **not** host, distribute, or contain any copyrighted game assets, audio files, textures, proprietary game code, or DRM-circumvention tools belonging to EA, Pandemic Studios, or their rights holders.
* **Prerequisites & License Requirement:** To compile or use this project, you **must legally own a licensed copy** of *Mercenaries 2: World in Flames*. The game reads its data from your own legally purchased installation.
* **Anti-Piracy Policy:** The contributors to this project strictly oppose software piracy. We do not support, condone, or provide assistance with illegal downloads, ROM/ISO sharing, or illegitimate copies of the game.

*Mercenaries 2: World in Flames* is a trademark of Electronic Arts Inc. This project is not affiliated with or endorsed by Electronic Arts or Pandemic Studios.

<sub>Licensed under MIT or Apache-2.0, at your option. The license covers this repository's own code only.</sub>
