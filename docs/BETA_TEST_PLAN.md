# Beta test plan

Repeatable checks for the exit criteria in [BETA_READINESS.md](BETA_READINESS.md)
(E1 to E13), feeding the items in [BETA_ROADMAP.md](BETA_ROADMAP.md) (R1,
R2, ...). Tests are numbered T0 to T14. Each test says which criteria it
decides, whether it is **SP** (both release targets) or **Co-op** (co-op
beta only), and whether it is automated, needs a human, or both.

Automated runs follow the PC's run rule: at most three game runs at once,
each at low priority. All game data is read from the player's own install
at run time; nothing from the game is copied into the repository, and
screenshots and reference frames stay on the test PC.

`<install>` is the game's install folder. `m2-alpha` writes its log to
stderr, so the evidence lines below are grepped from the run's `.err` file.

## Two kinds of contract test

| | Contract in isolation | Normal story progression |
|---|---|---|
| How it starts | `--skip-to <id>`: the scripts' developer skip plays the story up to the contract | A new game (or a T2b save) with the scripts' skip mode off: `--movies`, no `--skip-to` |
| What it proves | The contract's own script can reach `Pg.ContractCompleted` in the alpha | The contract is offered when the flow unlocks it, plays, and its consequences carry on |
| Criterion | E5 (T6) | E6 and E7 (T2b) |

A completion in isolation is **never** counted as story progression. A
headless `--mission` run without `--movies` keeps the scripts' skip mode on
for the whole session [code: `mission.rs` `with_skip_mode_off`; PR #158],
so such a run is an isolation run even when it starts from a new game.
A run with a contract-specific `--mission-driver` satisfies neither E5 nor
E6; it may be recorded as a diagnostic only.

## Prerequisites

- Windows 10 or 11 x64 with the game installed (the alpha finds it through
  the registry, or `--install <install>`).
- Rust toolchain from `rust-toolchain.toml` (channel 1.97, `rustfmt`,
  `clippy`); targets `x86_64-pc-windows-msvc` (host) and
  `i686-pc-windows-msvc` (installed by the toolchain file).
- Visual Studio Build Tools with the C++ workload, x64 and x86 libraries:
  the vendored Lua in `m2-game` is compiled with the MSVC C compiler.
- `nvidia-smi` (or the GPU vendor's equivalent) for GPU utilization in T9
  and T12.
- For T11: a second PC (or the `coop_peer` example for the automated part).
- For T14: a Windows user account that has never had the repository, Rust
  or Visual Studio on its path.

## Recording results

Every result, automated or human, is one row in the candidate's results
file (kept on the test PC beside the logs; only numbers and short log lines
go into reports):

| Field | Example |
|---|---|
| Candidate commit SHA | `de7ea9d` |
| Build configuration | `release`, `x86_64-pc-windows-msvc`, `RUSTFLAGS=-D warnings`, incremental on |
| Test name | `T6 ChiCon003` |
| Arguments | the full command line after the exe |
| Run number | `2 of 3` |
| Exit code | `0`, or `killed at limit` |
| Result | pass / fail / waived, and the evidence line (`Pg.ContractCompleted at 1826 s`) |
| Log location | the run's folder (`.err`, `.log`, mission and HUD logs) |

Performance results (T5d, T9, T12) also record: CPU, GPU and driver
version, RAM, display resolution and window mode, the graphics settings in
`m2-alpha.toml` (view distance, shadows, vsync, frame cap), other programs
running, the warm-up period, the route, and the measurement duration.

## T0. Build gate (E1, E2, E3) — SP, automated

Run in a fresh worktree of the candidate commit:

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo test -p m2-game --target i686-pc-windows-msvc
cargo clippy --workspace --all-targets --target i686-pc-windows-msvc -- -D warnings
cargo build --workspace --target i686-pc-windows-msvc
cargo xtask symbols
cargo build --release -p m2-alpha
```

Pass: all exit 0 (E1), the CI workflow is green on the same commit (E2),
and the Windows x64 and i686 builds and tests pass (E3; Windows is the
supported platform). A Linux `cargo test --workspace` is best-effort: run
it when a Linux host is available and record the result, but it does not
decide E3. Record test totals. For E2, record each PR merged in the 7 days
before the candidate as "pass", "fail" or "skipped (paths)": a docs-only
PR that the path filter skips is not a failure.

## T1. Fresh launch and start-up (E4) — SP

- **Automated:** 20 cold launches with `--to-front-end`, each killed 60 s
  after the front end appears. Evidence: the front end's first-frame line
  and launch-to-front-end seconds. Pass: 20 of 20, all under 60 s. Then 3
  new games from the front end to control of the hero.
- **Human:** the attract video, the menus with mouse, keyboard and pad, and
  options kept after a restart. (The clean-account version is T14.)

## T2. Story

### T2a. Opening contracts (E5 diagnostic) — SP, automated

`--mission --autoplay --mission-seconds 1500 --after-contract 300` from a
new game. Evidence: `Pg.ContractCompleted` for VzaCon001, then PmcCon001
offered. A quick smoke test only: skip mode stays on, so it does not count
for E6.

### T2b. Story progression without developer skip (E6, E7) — SP

The end-to-end test. It starts from a new game with skip mode off and plays
the story in order, saving and reloading at each boundary, until the final
required story contract.

**Boundaries.** The story spine is the order of `WifMissionFlow`'s
bindings [code: flow listing]: `Start`, VzaCon001, PmcCon001, OilCon020,
OilCon021, OilJob011, PmcCon002 (with the mechanic and pilot intros and
MecCon001 / JetCon001), PmcCon003, `AllChiIntro`, PmcCon004. Faction
contracts unlock from their own bindings along the way.

| Boundary | After | Check before saving |
|---|---|---|
| B0 | New game, control of the hero | No contract done; VzaCon001 active |
| B1 | VzaCon001 | Villa consequence layers; PmcCon001 offered |
| B2 | PmcCon001 | PMC HQ reached by the flow's move, not a teleport from skip; faction contracts offered as the flow unlocks them |
| B3 | OilCon020, OilCon021 (UP introduction) | OilCon050, PmcCon013, PmcCon034 and the UP jobs unlocked (`UnlockMission` in that step) |
| B4 | PmcCon002, MecCon001, JetCon001 | Mechanic and pilot join; PmcCon018 unlocked when the pilot joins |
| B5 | PmcCon003 | Its consequence: state layers swapped, HQ moves as the flow says |
| B6 | `AllChiIntro` | Allied and China starters and their first contracts offered |
| B7 | Every contract that PmcCon004's consequence destroys, plus the remaining faction contracts the run plays | AllCon001, 002, 003, 008, 050, ChiCon001, 002, 003, 008, 050 and GurCon003 done (they cannot be played after B8); `HasKey("PmcCon004")` true, so PmcCon004 is offered |
| **B8** | **PmcCon004, the final story contract** | `Pg.ContractCompleted` for PmcCon004, then the `WifMissionFlow` binding `PmcCon004`'s consequence runs to its end: Solano verified, the ending movie `15_ACK_` plays, the credits widget shows, the forced autosave is written, and the game requests `unloading` and returns to the front end. After that, Continue from the autosave loads without error |

B8 is the qualifying end: a T2b chain passes E6 only when B8's checks all
pass. Contracts outside the spine that are not destroyed at B8 (the PMC
challenges, races, outposts and jobs) are covered by E5 in isolation;
whether the game lets them be played after the credits is not yet known
[unverified] and is recorded at B8.

**Automated chain.** One run per boundary, each loading the previous
boundary's save:

```
m2-alpha --install <install> --mission --movies --autoplay \
  --contracts <n> --mission-seconds <limit> \
  [--load <previous boundary save>] --save-to <this boundary save> \
  --size 1280x720 --screenshot <out>
```

`--contracts <n>` has autoplay play the first contract and then the story's
next ones [code: `main.rs`]. No `--skip-to`, no `--mission-driver`. After
each run:

1. **Availability:** the scripts' save line (`N done [...], active [...]`)
   lists exactly the contracts expected at this boundary; contracts the
   flow has not unlocked are not offered.
2. **Consequences:** the state layers the contract's consequence adds or
   marks for removal are loaded or gone (mission log `MrxLayerManager`
   lines).
3. **Reload:** a fresh launch with `--load <this save>` reports the same
   save line, the same act, and the hero within 2 m of the saved place
   (E7).
4. **Lua growth:** the registry size line (R13) at the boundary, kept in
   the T9 ledger.

Pass (E6): every boundary through B8 passes 1 to 4 in one chain. A
contract autoplay cannot finish in the chain may be finished by a human
tester from the previous save, and the chain then continues from the
human's save; record which boundaries were human-assisted. A boundary that
needs `--skip-to` or a contract driver fails E6.

**Human:** the same boundaries played by hand at least once per candidate
(part of T13), including at least one save and reload across B5 and B6.

## T3. Free roam (E8a, E9, E6) — SP

- **T3a automated:** after a T2b boundary save, free roam for 15 minutes
  with autoplay driving between points across the island. Evidence:
  `pace:` lines, memory samples every 15 s, no panic. Feeds T9's ledger.
- **T3b human, layers per act:** at B2, B5 and B6, drive through Maracaibo
  villages, the coast road and Caracas. Check against the reference footage
  on the PC: people (soldiers, civilians) where the game has them, props
  and destroyed or repaired buildings as the act's state layers say,
  parked vehicles, trees beside roads. File each difference with a
  position and a footage frame number.
- **T3c human, visual:** day and night, lighting, grade, HUD edges (R22).

## T4. Vehicle acquisition and destruction — SP

- **Automated:** the outpost and hijack contracts (GurCon050, PmcCon004)
  exercise boarding and hijacks; covered by T6.
- **Human:** get into each class (car, truck, tank, boat, helicopter),
  hijack a tank and a helicopter with the pad and with the keyboard, call
  a vehicle delivery, wreck one of each class. Check brake and reverse
  lamps, engine sounds, the wreck's fire and smoke.

## T5. Weapons, explosions and audio — SP

- **T5a automated, satellite bomb:** `M2_SAT_AHEAD=<m>` at 45 m and 120 m;
  measure luminance and fireball size on the screenshots with the polish
  thread's scripts. Pass: within the bounds set from the footage in that
  thread's report (R17).
- **T5b human, effects:** every hand weapon, grenades, C4, an airstrike, a
  bunker buster, the satellite bomb. Check smoke near blasts (R20),
  particles following moving effects (R18), wind on smoke (R19), glow.
- **T5c human, audio:** with sound on (automated runs have none): 3D
  attenuation (a gunshot and an explosion at 10, 50 and 200 m, and moving
  left to right), engines up close and passing, radios fading over their
  reach, music changes in and out of combat, VO and subtitles in a
  briefing, pause holding all sound. Note every action with no sound or a
  wrong sound, with the position and what was done (feeds R21).
- **T5d particle-heavy performance:** three airstrikes in Caracas within
  10 s on the T12 settings; record `pace:` lines and memory for 60 s.

## T6. Contracts in isolation (E5) — SP

- **T6a automated:** all 52 flow contracts, 3 runs each on the candidate:

  ```
  m2-alpha --install <install> --mission --autoplay --mission-seconds 1500 \
    --skip-to <id> --size 1280x720 --screenshot <out>
  ```

  No `--mission-driver`. Evidence: `Pg.ContractCompleted` and the seconds
  it took. Pass: every contract 2 of 3 or better. The table below is the
  primary progress measure; update it in place.
- **T6b diagnostics:** a run with a contract driver may be made to locate
  a failure. It is recorded in the "Driver" column and never counted as a
  pass.
- **T6c human:** accept a contract from the HQ menu and from a contact in
  the field; fail one on purpose (timer, death, target lost) and retry it;
  check the PDA map, tracking and the pay credited on completion; finish
  one job of each kind (bounty, verify, destroy set, collection) (R23).

### Contract result table

"Passes / 3" is on one candidate commit and has not been run yet for any
contract; it replaces the historical "completed at least once" as the
measure. "History" is the latest PR or thread evidence on `main` (or the
named open PR), not re-run. "Driver" is whether the historical completion
needed a contract-specific driver. "Skip" is how the historical run
started. "Story" is whether it completed in T2b (no contract has yet).

| Contract | Passes / 3 | History | Driver | Skip | Story | Outstanding |
|---|---|---|---|---|---|---|
| VzaCon001 | - | completed (#138; this assessment's run) | no | new game, skip mode on | no | - |
| PmcCon001 | - | completed (#129) | no | not recorded | no | - |
| PmcCon002 | - | 1 pass, 1932 s (#141) | no | `--skip-to` | no | - |
| PmcCon003 | - | 1 pass, 1136 s (#141) | **yes** (race, tank waves) | `--skip-to` | no | Race stage and tank waves without driver |
| PmcCon004 | - | 1 pass, 458 s (#160) | **yes** (climb) | `--skip-to` | no | Plain autoplay stalls in a gully at (-2340, 200) |
| PmcCon013 | - | never run | - | - | no | Helicopter challenge [code] |
| PmcCon015 | - | never run | - | - | no | Race [code] |
| PmcCon016 | - | 7 of 27 gates | - | `--skip-to` | no | Race |
| PmcCon018 | - | never run | - | - | no | Airstrike challenge [code]; opens after JetCon001 |
| PmcCon031 | - | 3 of 3 (#162) | no | `--skip-to` | no | - |
| PmcCon032 | - | never completed | - | `--skip-to` | no | WIP fix on a local branch; round 45 unrun |
| PmcCon033 | - | 4 of 4 (#153), 1 more on #162 | no | `--skip-to` | no | - |
| PmcCon034 | - | 1 of 2 (#157, #162) | no | `--skip-to` | no | Time check needs truck busts to fall off |
| MecCon001 | - | 1 pass, 263 s (#152, open) | **yes** (race) | `--skip-to` | no | Races without driver |
| JetCon001 | - | 1 pass, 2084 s (#152, open) | no | `--skip-to` | no | - |
| OilCon001 | - | 1 pass, 1717 s (#154) | no | `--skip-to` | no | - |
| OilCon002 | - | 2 of 3 (#158) | no | `--skip-to`, `--movies` | no | - |
| OilCon003 | - | passes at 370-482 s, one timeout (#154) | no | `--skip-to` | no | Timed |
| OilCon005 | - | never completed | - | `--skip-to` | no | Car race: timer out at gate 2 |
| OilCon020 | - | 1 pass, 1767 s (#154) | no | `--skip-to` | no | One run lost the goods truck to a bump |
| OilCon021 | - | passes at 261-311 s (#154), 263 s (#158) | no | `--skip-to` | no | - |
| OilCon050 | - | 1 pass, 718 s (#154) | no | `--skip-to` | no | - |
| OilCon051 | - | 1 pass, 701 s (#154) | no | `--skip-to` | no | - |
| OilCon052 | - | 1 pass, 793 s (#154) | no | `--skip-to` | no | - |
| GurCon001 | - | completed (#137) | not reported | `--skip-to` | no | - |
| GurCon002 | - | completed (#137) | not reported | `--skip-to` | no | - |
| GurCon003 | - | 1 pass, 382 s, 21/21 gates (#137) | not reported | `--skip-to` | no | Boat race |
| GurCon005 | - | completed (#137) | not reported | `--skip-to` | no | - |
| GurCon050 | - | completed (#137) | not reported | `--skip-to` | no | - |
| GurCon052 | - | completed (#137) | not reported | `--skip-to` | no | - |
| GurCon053 | - | completed (#137) | not reported | `--skip-to` | no | - |
| PirCon001 | - | never completed | - | `--skip-to` | no | Jetski catches the harbour pier near checkpoint 2 |
| PirCon002 | - | 1 pass, 807 s (#137) | not reported | `--skip-to` | no | - |
| PirCon003 | - | 1 pass, 363 s (#137) | not reported | `--skip-to` | no | - |
| PirCon004 | - | 1 pass, 623 s (#137) | not reported | `--skip-to` | no | - |
| PirCon051 | - | completed (#137) | not reported | `--skip-to` | no | - |
| PirCon052 | - | completed (#137) | not reported | `--skip-to` | no | - |
| AllCon001 | - | 1 pass, 2485.6 s (#140) | no | `--skip-to` | no | - |
| AllCon002 | - | never completed | - | `--skip-to` | no | MLRS race won in some runs; boats stage lost |
| AllCon003 | - | 1 pass, 1141.5 s (#140) | no | `--skip-to` | no | - |
| AllCon008 | - | never run | - | - | no | Unlocked after AllCon052 only before PmcCon004; destroyed by PmcCon004's consequence [code] |
| AllCon050 | - | 1 pass, 646.8 s (#140) | no | `--skip-to` | no | - |
| AllCon052 | - | 1 pass, 1405.4 s (#140) | no | `--skip-to` | no | - |
| AllCon053 | - | 1 pass, 658.3 s (#140) | no | `--skip-to` | no | - |
| ChiCon001 | - | completed (#144) | no | `--skip-to` | no | - |
| ChiCon002 | - | 14 of 15 depot targets | - | `--skip-to` | no | Last depot target |
| ChiCon003 | - | 1 pass, 1826 s (#151) | no | `--skip-to` | no | - |
| ChiCon008 | - | never completed | - | `--skip-to` | no | Tank race: waits at an optional barrel |
| ChiCon009 | - | 1 pass, about 310 s (#151) | no | `--skip-to` | no | - |
| ChiCon050 | - | completed (#142) | no | `--skip-to` | no | - |
| ChiCon051 | - | never surveyed | - | - | no | - |
| ChiCon053 | - | never surveyed | - | - | no | - |

Historical totals: 37 completed on `main` (35 without a driver), 2 in #152
(1 without a driver), 13 never; 0 with a 3-run record on one commit; 0 in
story progression.

## T7. Recruit progression — SP

- **Automated:** MecCon001 and JetCon001 in T6 (evidence: the recruit flag,
  `mechanic false -> true`, and the `MecPmcBoss` / `JetPmcBoss` starters),
  and again inside T2b at their boundary.
- **Human:** after each recruit joins, check what it unlocks (the
  mechanic's vehicles, the pilot's deliveries, PmcCon018) in the shop and
  the PDA.

## T8. Save, quit and reload (E7) — SP

- **Automated:** covered at every T2b boundary (step 3).
- **Human:** save from the pause menu, Quit to main menu, Continue; quit
  during an autosave; load an older save after a newer one; save just
  before and just after B5 and B6 and load each in a fresh launch.

## T9. Extended play and the stability ledger (E8a, E8b, E9) — SP

- **T9a automated, 60-minute session (E8a):** one contract then free roam
  for 60 minutes in total, with memory sampled every 15 s and the Lua
  registry line (R13) at each `pace:` line. Pass: no panic, no hang (R15's
  watchdog: no frame or tick for 10 s), memory growth under 10 % between
  minute 10 and minute 60 (E9), no steady growth in the Lua registry.
- **T9b transitions:** 20 cycles of contract start, contract end, save and
  load in one session (T2b's chain or a loop over one boundary save).
  Record the Lua registry size after each cycle. Pass: no growth trend
  across cycles.
- **T9c human:** a 2-hour session by a tester, noting slowdowns, audio
  dropouts or visual corruption with the time.
- **Ledger (E8b):** every automated and human run on the candidate adds its
  wall-clock hours and its crash and hang count to one ledger. Pass: at
  least 20 cumulative test-hours on the candidate and at most 1 failure
  per 10 hours, where a failure is a crash or a hang, counted together. A
  crash or a hang in T9a fails E8a regardless of the aggregate.

| Candidate | Runs | Test-hours | Crashes | Hangs | (Crashes + hangs) per 10 h |
|---|---|---|---|---|---|
| (filled per candidate) | | | | | |

## T10. Controls (E10) — SP, human

One tester per input device, ticking every action in each context:

- **Keyboard and mouse:** on foot (move, sprint, crouch, jump, melee,
  reload, fire, aim and zoom, switch weapon, throw, action/use, support
  menus, PDA, pause), every vehicle class (drive, brake, handbrake, horn,
  exit, fire, camera), aircraft (lift, yaw, fire, exit), boats, hijacks
  (the face-button prompts), menus, the PDA map (pan, zoom, track), and
  rebinding a key in the options.
- **Pad (Xbox layout):** the same list against the game's own pad layout
  (`pad.rs` table), plus vibration and button pictures in prompts.
- **Mixed:** switch device mid-play; the prompts change to that device.

Pass: every action works in every context with each device, or has a
written waiver.

## T11. Co-op (E11) — Co-op only

- **Automated:** `coop_peer` (`--host` / `--join`, `--people N`) against
  the alpha with `--coop-seconds 600`. Evidence: both peers log each
  other's positions; no protocol errors. Pass: 10 minutes with no desync
  warning.
- **Human (two PCs):** host and join through the join page; play one
  contract together for 30 minutes. Pass (E11): the contract completes on
  both screens, positions agree within the protocol's tolerance throughout,
  no crash. Never paste an invite code in a public place; it carries the
  host's public address.

## T12. Controlled performance benchmark (E9) — SP, automated

The figure E9 is judged on. Same PC for every candidate; nothing else
running.

- **Settings:** 1920x1080 borderless; `m2-alpha.toml` graphics settings
  fixed and recorded (view distance, shadows, vsync off, frame cap off);
  the same driver version as the baseline.
- **Route:** a fixed save at B2 (PMC HQ) and a fixed autoplay drive from
  the HQ into Caracas by the coast road, the same each time (the route's
  points kept with the results).
- **Timing:** 60 s warm-up after control of the hero (not measured), then
  10 minutes measured.
- **Evidence:** `pace:` lines with the per-part breakdown from R16 (tick,
  scripts, streaming, draw, present), CPU and GPU utilization and memory
  every 15 s.
- **Pass:** median at least 60 fps; fewer than 60 frames over 50 ms per
  minute. Compare each candidate with the previous one on the same route;
  a drop of more than 10 % is a regression to explain before release.

## T13. Human playtest and report template (E12) — SP (+ Co-op)

One complete session per act (B0-B2, B2-B5, B5-B8), plus a co-op session
for the co-op beta, each covering the human parts of T2b, T3-T8 and T10 it
reaches. Testers cover each mission type at least once by hand: race,
timed defence, escort and follower, hijack, sniping, delivery, outpost
capture, HVT verify, bombing and airstrike, helicopter flight.

Each finding uses this template:

```
Build SHA:        de7ea9d (from the title bar or the log's first line)
Release target:   single-player / co-op
Session:          act, boundary save used, date
Area:             T-number (for example T5c)
Severity:         blocker / major / minor / cosmetic
Summary:          one line
Steps:            1. ... 2. ... 3. ...
Expected:         what the game does (footage frame if visual)
Actual:           what the alpha does
Log lines:        a few lines from m2-alpha's log, with the time
Disposition:      open / fixed in <commit> / waived (reason, by whom)
```

Pass (E12): every blocker is fixed or waived in writing.

## T14. Release package on a clean account (E13) — SP

On a Windows account (or a clean installation) with no repository, Rust,
Visual Studio or project folders on it, and the game installed normally:

1. Download the release archive the way a tester would and unzip it into a
   new folder.
2. Launch with no arguments. Pass: the install is found without
   `--install`, the front end appears.
3. Configuration: change two options (resolution, volume), quit, relaunch.
   Pass: both kept; the settings file is written beside the exe (or where
   the README says).
4. Saves: start a new game, save, quit, relaunch, Continue. Pass: the save
   file exists and loads.
5. Clean shutdown: quit from the menu. Pass: the process exits with code 0
   within 10 s, no crash log written.
6. Missing install: rename access to the install (or run on an account
   without it). Pass: a clear message, no crash.

## Human versus automated

Autoplay can prove that scripts reach completion, that the game doesn't
crash or leak, and give frame-rate and memory numbers. It cannot judge how
the game looks against footage, how it sounds (test runs start no audio),
how controls feel, whether a human can finish races and timed stages,
menus by hand, and co-op between two people. Those are T3b-c, T4, T5b-c,
T6c, T8, T10, T11 and T13's human parts, run once per release candidate.

## Release candidate checklist

Fill one row per criterion with **pass**, **fail** or **waived** (reason,
owner, date), the candidate SHA, and the result rows it rests on. A
candidate ships only when every criterion of its target is pass or waived.

| Criterion | Target | Test | Result | Evidence / waiver |
|---|---|---|---|---|
| E1 Clean build | SP | T0 | | |
| E2 CI green | SP | T0 | | |
| E3 Platform support (Windows x64 and i686) | SP | T0 | | |
| E4 Start-up, 20 cold launches | SP | T1 | | |
| E5 Contracts in isolation, 52 at 2 of 3, no drivers | SP | T6 | | |
| E6 Story progression without skip | SP | T2b | | |
| E7 Save and load at every boundary | SP | T2b, T8 | | |
| E8a 60-minute session | SP | T9a | | |
| E8b Crash rate over at least 20 test-hours | SP | T9 ledger | | |
| E9 Performance, controlled benchmark | SP | T12, T9a | | |
| E10 Controls, both devices | SP | T10 | | |
| E11 Co-op | Co-op | T11 | n/a for single-player beta | |
| E12 Human playtest | SP (+ Co-op) | T13 | | |
| E13 Release package on a clean account | SP | T14 | | |

For a single-player beta, E11 is recorded as "excluded: single-player
release" and the release notes say co-op is not supported.
