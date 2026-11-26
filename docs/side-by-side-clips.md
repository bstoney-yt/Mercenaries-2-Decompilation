# Side-by-side clips

One fix, one case file, one clip: `m2-alpha` ("Ours") next to the Xbox 360
game in Xenia, the same moment lined up, for release notes. The tool is
`tools/xenia-compare/clips.py` (Python standard library + a local ffmpeg);
it builds on the Xenia tooling there (`xc.py` routes, the `xpad-shim` pad,
`--mission-shots`). See `tools/xenia-compare/README.md` for Xenia's setup.

**Footage stays on the LAN.** Clips, frames and logs go under
`Z:\mercs2\scratch\clips\<case>\` and are never committed or uploaded;
Brendan decides what is posted. Case files and routes hold no game data.

## A case

`tools/xenia-compare/cases/<name>.json`:

```json
{
  "name": "crash-toss",
  "fix": "#723",
  "caption": "Crashing a car knocks her down and she gets up by the 360's rule",
  "summary": "Optional line for the release notes.",
  "start_state": "Free text: where both sides start.",
  "duration": 14,
  "fps": 30,
  "size": "640x360",
  "slowmo": 2.0,
  "swap": false,
  "xenia": {
    "routes": ["clip-oilcon020-wreck.txt"],
    "start": "mark:crash",
    "frames": "after",
    "capture": "video",
    "args": ["--headless=true"],
    "audio": true,
    "offset": 0.0
  },
  "ours": {
    "args": ["--mission", "--skip-to", "OilCon020", "--autoplay", "--mission-seconds", "120"],
    "shots": {"from": 0, "to": 100, "fps": 10},
    "start": "log:autoplay ([\\d.]+) s: .*(?:wrecked|crash)",
    "wall": 900,
    "offset": 0.0
  }
}
```

| field | meaning |
|---|---|
| `duration` | seconds of play in the clip, from each side's start event (slow motion lengthens the clip, not this) |
| `fps`, `size` | the clip's rate and each half's size (even numbers); both sides are scaled and padded to it |
| `slowmo` | 2 plays at half speed |
| `swap` | Xenia on the left (default: Ours left) |
| `xenia.routes` | `xc.py` route files, run one after another (`routes/` or beside the case) |
| `xenia.start` | the shared start event on Xenia's side: `mark:<label>` (a route `mark`), `shot:<name>`, or `t:<seconds>` |
| `xenia.capture` | `video`: record Xenia's window with ffmpeg (`gdigrab`) while the route plays; `frames`: the route's own screenshots |
| `xenia.frames` | the shot-name prefix used for the fallback frames (`burst after 8 1` makes `after-000`...) |
| `ours.args` | m2-alpha's arguments (it is run offscreen: `--install`, `--size` and `--screenshot` are added) |
| `ours.shots` | dense `--mission-shots` from `from` to `to` seconds of play at `fps` (at most 10: m2-alpha names shots to 0.1 s) |
| `ours.start` | the start event on our side: `log:<regex>` on m2-alpha's stderr (group 1 = seconds of play), or `t:<seconds>` |
| `*.offset` | a frame-sync nudge, seconds, added to that side's start |

Examples: `airdrop-vzacon001.json` (#715, VzaCon001's first crate; the best
first trial, its autoplay timing is known), `cargo-parked-truck.json`
(#776), `crash-toss.json` (#723). Their start regexes and offsets for the
last two are first guesses: tune them from the first capture's logs
(`ours/m2.err`, `xenia/route.log`) and recompose with `compose`, no new run
needed.

## Commands

```powershell
cd tools\xenia-compare
python -I clips.py check cases\crash-toss.json          # the case is valid; the --mission-shots it will pass
python -I clips.py notes cases\crash-toss.json          # the release-notes snippet
# Both sides, then the clip (needs the GPU slot; see below):
$env:XENIA_GAME = '<path to the extracted default.xex>'
python -I clips.py capture cases\crash-toss.json --exe <m2-alpha.exe> --install '<install dir>'
# Only the clip, from two existing clips (no GPU, no game):
python -I clips.py compose cases\crash-toss.json --ours <ours.mp4|frames dir> --xenia <xenia.mp4> `
  --ours-start 31.2 --xenia-start 12.0 [--xenia-offset 0.1] [--slowmo 2]
```

`capture` writes `<out>\<case>\`: `xenia\` (route log, window recording or
shots), `ours\` (`m2-command.txt`, `m2.err`, `ours-frames.mp4`),
`side-by-side.mp4` (H.264, labelled, caption bar), `side-by-side.gif`,
`thumb.png`, `notes.md` and `compose.json` (the starts used, to recompose).
m2-alpha's frames are written to `C:\Claude\scratch\clips-tmp` (a local
disk) and deleted once they are a clip.

## Lining up

Each side's clip has its own clock. Xenia's route log now begins with
`t0 epoch <t>` (the wall clock its times count from); ffmpeg's `gdigrab`
stamps the recording with the wall clock (`start:`), so a route mark lands
at `t0 + mark - start` in the recording. With frames instead, a clip starts
at its first frame's time. Our side's frames carry their seconds of play
(`mission-shot: <s> s:`), and the start regex gives the event's. Both clips
are trimmed from their events for `duration`, then put side by side; a
start before a clip's first frame holds that frame.

## The GPU slot

`capture` refuses to run unless the slot is its own: it takes
`C:\Claude\scratch\gpu.lock` (the nightly runner's lock, same JSON: pid,
start time, purpose; a lock whose pid is gone or that is over 8 h old is
taken over) and then needs no `m2-alpha*` or `xenia*` program running for 3
checks in a row. It waits up to `--wait-minutes` (30) and otherwise exits
3 without starting anything. It runs Xenia, then m2-alpha, one at a time,
and releases the lock before composing. Only the threads the coordinator
hands the GPU to (the merge queue, the Xenia thread) run `capture`.

## Limits

- Window recording needs a connected desktop session (as `shot.ps1` does);
  from a disconnected one it records black, and the tool falls back to the
  route's screenshots (1 a second at most: Xenia names them to the second).
- Our frames come from `--mission-shots`, so a case's m2-alpha run must be
  a `--mission` run; the `--*-check` flags (`--toss-check`,
  `--gravity-check`, `--cargo-settle-check`...) simulate without drawing and
  give no frames. A clip of such a fix plays the same moment in the
  mission instead.
- At most 10 frames a second on our side (shot names to 0.1 s); the clip's
  rate holds each frame.
- The tank fall (#771) has no case yet: its Xenia route (ChiCon001) is not
  in `routes/`.

## PR rules

- A PR adding or changing a case states its **Pass condition in the PR body
  before any capture run** (for example: "the clip shows both sides from
  the crash mark within 0.2 s, and the crates stay on the bed in both").
- Docs-only changes get no CI; `clips.py` has its own tests:
  `python -I -m unittest discover -s tools/xenia-compare -p "test_*.py"`
  (synthetic clips made with ffmpeg's lavfi sources; skipped without ffmpeg).
