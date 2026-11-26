# Nightly autoplay

Every night the dev VM plays each contract of the story with
`m2-alpha --mission --autoplay`, one at a time, and flags what broke. The
runner is the `m2-nightly` crate; `tools/nightly-autoplay/` holds the
PowerShell that builds `origin/main`, runs it and registers the scheduled
task.

## What autoplay is (and what it can drive)

`--autoplay` (`crates/m2-alpha/src/autoplay.rs`) plays a contract with a
player's inputs, steered by the markers the game's own mission scripts put
up. Run with `--mission --screenshot <out>.ppm` it plays offscreen until the
contract completes or `--mission-seconds` are up. `--skip-to <id>` starts a
contract as the game's developer switch does (the story contracts before it
completed); `VzaCon001`, the first, plays from the start without it.
`--contracts 1` stops after one contract.

It can: run and drive to places by a route planned round walls, the sea and
steep ground; shoot marked soldiers and blow up marked targets (rifle,
rockets, emplaced guns, satellite bombs on marked gates); open and close the
PDA when the scripts wait for it; take the vehicles and boats the scripts
bring, ride helicopter transits, take action prompts, break supply crates
and pick up what they leave, wait out deliveries, and follow race
checkpoints.

It can't (today): fly aircraft (it gets out), play co-op, or choose
dialogue freely. Where it makes no headway for a while it **hops** ahead (a
test shortcut, logged and counted): a completed run with hops shows where
the way is blocked rather than proving the contract playable unaided.
Several conveniences are marked "test shortcut" in its log (followers
brought to her, beacons set without the PDA map).

## The runner

```
m2-nightly contracts --install <install dir>
m2-nightly run --exe <m2-alpha.exe> --install <install dir> --out <root> [--only A,B] [--deadline-unix T]
m2-nightly analyse <run.err> [--exit-code N] [--killed hang]
m2-nightly report <night.json>
m2-nightly issues --night <night.json> --repo <owner/name> [--known-bugs 698] [--dry-run]
```

**Contracts** are read at run time from the install's `data\vz.wad` index:
script modules named `<faction>Con<nnn>` (52 in the PC release; `--jobs`
adds the 20 `<faction>Job<nnn>`). Nights that can't fit every contract take
turns: `<root>/last-played.json` puts the least recently played first.

**Each run** gets `<root>/<night>/<id>/` with `<id>.err` / `<id>.log` (the
game's stderr and stdout), `result.json` and `summary.txt` (the detectors'
findings plus the `m2-runsum` summary). Pictures of passing runs are
deleted (`--keep-pictures` keeps them). The night writes `night.json` and
`report.md` (under 3 KB) as it goes, so a night cut short leaves results.

**Detectors** (`crates/m2-nightly/src/detect.rs`; every threshold is a
tooling heuristic, changeable by option):

| kind | from |
|---|---|
| `panic` | a `panicked at` line or exit code 101; the issue names our source location only |
| `crash` | any other abnormal exit code (access violation, stack overflow, abort...) |
| `error` | m2-alpha's own `Error:` exit (code 1) |
| `hang` | no log output for 300 s of wall time: the process is stopped |
| `stuck` | no objective or mission-state change for 600 s of play; checked live each minute, and the run stopped |
| `fell-through` | autoplay's own `fell through` line, her position not a number, or a logged height under -150 m |
| `fps-cliff` | 3 `pace:` lines in a row over 50 ms a tick (offscreen; healthy is about 10) or under 15 fps (windowed) |
| `timeout` | past the 2700 s wall limit: the process is stopped |
| `not-completed` | ended without the contract completed, nothing above explaining it |

One cause stands for the others it explains: a `panic`, `hang` or `stuck`
run files no `not-completed` issue as well. An `fps-cliff` always files its
own.

**GPU yield.** The runner holds `C:\Claude\scratch\gpu.lock` (JSON: pid,
start, purpose) for the night, and takes over a lock whose pid is gone or
that is older than 8 h. Before each contract it also keeps to the VM's slot
rule: no process named `m2-alpha*` (the Xenia launcher's exe is
`m2-alpha-xenia.exe` for this) or `xenia*` for 3 checks a minute apart. If
the lock or the slot stays held until the window closes, the contracts left
are recorded as skipped. Its own m2-alpha runs at below-normal priority
with no window.

**Window.** `--deadline-unix` (the wrapper's `-StopAt 07:00`) is a hard
stop: the run playing is stopped and recorded as `aborted` (not a failure),
no new run starts with under 5 minutes left, the lock is released.

## Issues

`m2-nightly issues` opens **one issue per distinct break**, keyed by
contract and kind (`PirCon001/stuck`, kept in a hidden marker in the body),
labelled `nightly-autoplay` and `bug`. A break seen again edits its issue's
body (first seen, last seen, times seen: no notification); a closed issue
whose break comes back is reopened with one comment; the same night run
twice changes nothing. The pinned known-bugs issue #698 gets **one** index
comment listing the open nightly issues, edited only when that list
changes.

Issue text is numbers, kinds, contract ids, commit ids and our own source
locations only: no log lines, game text, pictures, footage or VM paths.
`--dry-run` prints what would be posted; `--offline` also skips reading
GitHub.

## Running it

The one-contract trial (no build: an exe and m2-nightly built from the PR
head), from the repository root:

```powershell
.\tools\nightly-autoplay\nightly.ps1 -Exe <target>\release\m2-alpha.exe -Nightly <target>\release\m2-nightly.exe -Head <sha> -Only PirCon001 -MaxMinutes 50 -Out Z:\mercs2\scratch\nightly\trial
```

The full night (builds `origin/main` first; what the scheduled task runs):

```powershell
.\tools\nightly-autoplay\nightly.ps1 -StopAt 07:00
```

The scheduled task (no admin: runs as the signed-in user, only while
signed in; daily 01:00, window closing 07:00, issues as a dry run until
`-PostIssues`):

```powershell
.\tools\nightly-autoplay\register-task.ps1
.\tools\nightly-autoplay\register-task.ps1 -PostIssues
.\tools\nightly-autoplay\register-task.ps1 -Unregister
```

What it needs: the game install (`-Install`, default the VM's); git and
cargo (a release build of `m2-alpha` and `m2-nightly` into
`C:\Claude\scratch\target-nightly`, about 3.5 min clean, the exes copied to
`C:\Claude\scratch\nightly-bin\<sha>`: the NAS share won't start an exe);
`gh` signed in as the repository owner (for issues); the NAS mapped as `Z:`
for output under `Z:\mercs2\scratch\nightly\` (about 1-2 MB of log per
contract, so roughly 100 MB a full night; pictures of failures on top). No
environment variables are needed. Nothing is deleted on the NAS: old nights
are left for whoever prunes them.

## PR rules for the runner and the breaks it finds

- A PR that fixes a nightly break states its **Pass condition in the PR
  body before any check run** (for example: "`PirCon001` completes with no
  `stuck` or `fell-through`, `m2-nightly analyse` exits 0").
- Only the merge-queue (game-check) thread launches m2-alpha for a check;
  other threads send the head and the exact command.
- Docs-only and tooling-only changes (`*.md`, `docs/**`) get no CI (the
  workflow's `paths-ignore`); Rust changes to `m2-nightly` do, like any
  crate.
