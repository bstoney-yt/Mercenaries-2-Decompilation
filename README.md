# Mercenaries 2 Decompilation Project

## Legal Disclaimer & Anti-Piracy Notice

* **Educational & Research Purpose:** This project is an open-source reverse-engineering effort created solely for educational, preservation, and interoperability purposes.
* **No Copyrighted Assets:** This repository does **not** host, distribute, or contain any copyrighted game assets, audio files, textures, proprietary game code, or DRM-circumvention tools belonging to EA, Pandemic Studios, or their rights holders.
* **Prerequisites & License Requirement:** To compile or use this project, you **must legally own a licensed copy** of *Mercenaries 2: World in Flames*. Users are required to dump/extract their own assets from their legally purchased retail disc or licensed digital copy.
* **Anti-Piracy Policy:** The contributors to this project strictly oppose software piracy. We do not support, condone, or provide assistance with illegal downloads, ROM/ISO sharing, or illegitimate copies of the game.

## Layout

A Cargo workspace targeting the original 32-bit process (`i686-pc-windows-msvc`), with everything that doesn't touch Windows also building on a 64-bit host.

| Crate | Purpose |
|---|---|
| `m2-layout` | `Ptr32<T>`, `Rva`/`Va`, and `assert_layout!` for binary-compatible structs |
| `m2-math` | D3DX-layout vectors and matrices, converted to `glam` |
| `m2-mem` | Ownership-based replacements for the original allocators |
| `m2-sys` | All raw FFI: Win32, Direct3D 9, original functions |
| `m2-hook` | Symbol tables and the per-function original/Rust switch |
| `m2-engine` | Reconstructed engine classes |
| `m2-game` | Gameplay logic |
| `m2-render` | Rendering abstraction over Direct3D 9 |
| `m2-assets` | Parsers for files from your own installation |
| `m2-modapi` | Data-driven mod definitions and hooks |
| `m2-oracle` | Differential tests against the original (opt-in, needs your exe) |
| `m2-loader` | The DLL loaded into the game process |

`symbols/` tracks every identified function (name, RVA, signature, translation status); see `symbols/README.md`. Conventions for translating code are in `docs/conventions.md`.

```sh
cargo test --workspace
cargo xtask symbols                                   # validate symbol tables
cargo clippy --workspace --target i686-pc-windows-msvc
cargo run -p m2-assets --example wad -- list "<install dir>\data\Loading.wad"
cargo run --release -p m2-assets --example gltf -- rigs "<install dir>\data\vz.wad"
cargo run --release -p m2-assets --example gltf -- export "<install dir>\data\vz.wad" 15d89832 dumps/jen.glb --filter jennifer
cargo run --release -p m2-assets --example gltf -- view dumps/jen.glb   # open http://127.0.0.1:8765
cargo run --release -p m2-assets --example gltf -- export "<install dir>\data\vz.wad" 15d89832 dumps/play.glb --filter resident --clips 1000   # then view it and tick Play mode (WASD, Shift, Q)
cargo run --release -p m2-assets --example gltf -- world "<install dir>\data\vz.wad" dumps/world.glb --detail 450,-1950 --radius 500   # terrain, buildings, props, ground scatter and 6 drivable vehicles (--objects static|all|none, --vehicles N)
cargo run --release -p m2-assets --example gltf -- view dumps/play.glb --world dumps/world.glb   # open http://127.0.0.1:8765/?spawn=450,-1950 for the town (&view=M, &veg=M draw distances); buildings and props collide; E gets in a vehicle
cargo run --release -p m2-assets --example wad -- world "<install dir>\data\vz.wad"   # check every terrain cell, tile, entity layer, scatter asset and model collision (PHY2)
```

