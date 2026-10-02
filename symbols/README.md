# Symbol tables

One TOML file per subsystem (`math.toml`, `memory.toml`, ...). Each entry
describes one function in the original executable: where it is, how it is
called and how far its translation has got. These files hold names, addresses
and signatures only, never decompiled code.

```toml
[[function]]
name = "Vector3::Length"          # unique across all files
rva = 0x1234                      # offset from the image base, hex
convention = "thiscall"           # cdecl | stdcall | thiscall | fastcall | custom
signature = "float __thiscall Vector3::Length(Vector3 *this)"
status = "named"                  # found | named | translated | verified
rust = "m2_math::RawVec3::length" # required once status is translated or verified
notes = "x87; returns in st(0)"   # optional
```

`cargo xtask symbols` validates every file and prints a progress summary. The
same check runs in the test suite.
