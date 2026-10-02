# Translation conventions

## Inputs and what gets committed

Commit Rust code plus names, addresses and signatures in `symbols/`. Do not
commit the executable, decompiler databases, or raw Ghidra/IDA output;
`.gitignore` blocks the common file types, and `dumps/` and `local/` are
ignored for scratch work.

## Target

The original is a 32-bit PE, so anything that runs inside the game process
builds for `i686-pc-windows-msvc`. Everything else (layouts, math, parsers,
symbol tooling) also builds and tests on a 64-bit Linux host.

## Raw layouts

- Mirror each recovered struct as `#[repr(C)]` in a `raw` module and pin it
  with `m2_layout::assert_layout!` (size and every known field offset).
- Store pointers as `Ptr32<T>`, never `*mut T`, so the layout is identical on
  32- and 64-bit hosts.
- Name unknown fields by offset (`unk_0x14: u32`) and padding explicitly
  (`_pad_0x1c: [u8; 4]`).
- Embed `m2_math::Raw*` types, not glam types, in raw structs.

## Classes and vtables

C++ vtables become explicit `#[repr(C)]` structs of `unsafe extern "thiscall"`
function pointers. The safe API is a Rust type or trait layered on top; Rust
trait objects never stand in for C++ objects.

## Calling conventions

Use the real convention (`thiscall`, `stdcall`, `fastcall`, `C`), all of which
are stable for i686. Record it in the symbol entry. Functions with custom
register usage need a naked-function shim; note the registers in `notes`.

## Unsafe

Every `unsafe` block gets a `// SAFETY:` comment saying what the original code
guarantees. Keep raw FFI in `m2-sys`; higher crates expose safe functions.

## Floating point

The original may compute in x87 with 80-bit intermediates; Rust on i686 uses
SSE2. Differential tests in `m2-oracle` compare with a per-function tolerance,
and functions that must be bit-exact (physics, saves) are flagged in `notes`.
