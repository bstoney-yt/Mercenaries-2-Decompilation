# Xenia vs m2-alpha side-by-side comparisons

Tools for putting the Xbox 360 game (running in Xenia Canary, from your own
disc) next to `m2-alpha` at the same moment, with nobody at the keyboard and
no desktop session needed. Nothing here contains or sends game data: frames,
audio and logs are written only to the output folder you name.

| File | What it does |
|---|---|
| `xpad-shim/` | `xinput1_4.dll` stand-in. Put it beside the Xenia exe and pad 0 reads its buttons and sticks from a shared-memory block (`Local\xpad-shim`) that `xc.py` writes. No driver or ViGEm. `XPAD_SHIM_LOG=<file>` logs the XInput calls every 2 s. |
| `xc.py` | Starts Xenia (`--hid=xinput`), runs a route file of pad steps, takes guest-frame screenshots (Xenia's own *Take Screenshot*, sent as a menu message, so it works from a disconnected session) and reads text with Windows' on-device OCR. |
| `xenia-route.ps1` | Waits until no `m2-alpha*`/`xenia*` GPU program has been seen for 3 checks, then runs `xc.py` through `uv`. `-Audio` adds SDL disk audio; `-XeniaArgs` passes Xenia options. |
| `m2-capture.ps1` | The same wait, then `m2-alpha --screenshot` (offscreen, `WGPU_BACKEND=dx12`) with your arguments, frames to `<case>\m2\`. |
| `xmem.py` | Reads Xenia's guest memory from outside (read-only `ReadProcessMemory`; guest address A is host `0x100000000 + A`, big endian). `scan x y z` finds float triplets near a point; `Tracker` and `MatrixTracker` follow a vehicle for `track`/`goto`. |
| `sheet.py` | Pairs `<case>\xenia\*.png` with `<case>\m2\*` in name order into `sheet.png` (Xenia left, m2-alpha right), plus `metrics.md` (luma, RGB, saturation, contrast, edge density, top/bottom luma per frame). |
| `routes/` | Route files: `vzacon001-boat.txt` (boot to the first contract's speedboat) and the case routes. |

## Setup (once)

```powershell
cd tools\xenia-compare\xpad-shim
cargo build --release --target x86_64-pc-windows-msvc
copy target\x86_64-pc-windows-msvc\release\xinput1_4.dll <xenia dir>\
```

Python 3.12 and `uv` are needed; `uv` fetches Pillow, numpy and the
`winrt-Windows.*` OCR packages on first use.

## Running a case

```powershell
$env:XENIA_GAME = '<path to the extracted default.xex>'
# Xenia: boot -> speedboat -> case steps (about 4 minutes to the boat)
Get-Content routes\vzacon001-boat.txt, routes\case-a-airdrop.txt | Set-Content <case>\xenia-route.txt
.\xenia-route.ps1 -Audio -XeniaArgs '--headless=true' -Route <case>\xenia-route.txt -Out <case>\xenia
# m2-alpha at the same spot
.\m2-capture.ps1 -Exe <m2-alpha.exe> -Install '<install dir>' -Out <case> `
  -GameArgs @('--hero','mattias','--mission','--autoplay','--mission-seconds','260','--mission-shots','5,10,15')
uv run --no-project --with pillow --with numpy python -I sheet.py <case>
```

## Things that matter

- **`--headless=true`**: with no Xenia profile, the game's sign-in prompt makes
  Xenia open its own host dialogs (sign-in, message boxes), which a guest
  screenshot cannot show and the pad cannot reliably answer. Headless mode
  answers them with their defaults. Then *Continue without saving* is the
  **upper** option (press up, then A), followed by an Xbox LIVE notice (A).
- **Audio**: with no sound device (a disconnected remote session) XAudio2
  fails and the game waits on a black screen. Use `-Audio` (`--apu=sdl` with
  SDL's disk driver): 6-channel float32 48 kHz PCM to `<out>\audio.raw`.
- **Screenshots** are named by Xenia to the second: keep shots 1 s apart.
- **Movies** skip with A; the route taps A until the boat's control hint
  ("Accelerate") is read.
- **Steering** without looking: `seek <rrggbb> ...` steers at the HUD
  objective marker's colour (the scripts give `HUD_objective_deliverable`
  255,200,0) and brakes when the marker stays off-centre (circling it).
- **Guest memory**: world coordinates in the 360's memory are the same as
  m2-alpha's (the speedboat starts at about (-1727, -36, 2069) in both).
  Vehicles have row-major 4x4 world matrices (rows 0..2 rotation, row 3 the
  translation with w = 1) in several copies, some refreshed only now and then.
  `track` picks the copies that move forward when A is pressed. `goto` steers
  at a point (heading from the last second's travel, else matrix row 2).
- **Speedboat controls on the 360**: A is the throttle (RT does nothing), X
  is brake/reverse. Stick right lowers the bearing atan2(dx, dz).
- `m2-alpha --mission-shots <s,...>` pictures the scene (no HUD) at those
  seconds of play, `<out>-t<s>.ppm`.
- One GPU program at a time: both launchers wait for the slot.

## Route steps

See the docstring at the top of `xc.py` (`wait`, `press`, `hold`, `stick`,
`move`, `set`/`release`, `shot`, `burst`, `burstmove`, `until`, `tap_until`,
`seek`, `ocr`, `mark`, `listen <file>` for live steering from another shell).
