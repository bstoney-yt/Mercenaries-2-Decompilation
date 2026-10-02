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
```

