# Beta roadmap

The prioritized plan from [BETA_READINESS.md](BETA_READINESS.md) (state of
`main` at `de7ea9d`, 2026-10-26, rechecked the same day). Each item is
numbered R1, R2, ..., names the exit criteria it serves (E1 to E13), its
test in [BETA_TEST_PLAN.md](BETA_TEST_PLAN.md) (T0, T1, ...), the
measurement that closes it, and its evidence. Items are meant to become one
PR each, in the established worktree and PR workflow.

Each item is marked **SP** (needed by both the single-player beta and the
co-op beta) or **Co-op** (needed by the co-op beta only). If co-op is
deferred, the single-player beta is: every SP item in Tier 1 done, every SP
criterion passed or waived, and co-op explicitly excluded from the release
(BETA_READINESS.md, "Release targets").

Order within a tier is the suggested order. R1 to R4 come first because
every later item's "done" depends on a build that CI can check.

## Tier 1: must fix before beta

These prevent reliable building, testing, progression or play. The core
single-player blockers are: CI and platform tests (R1-R4), contract
completion (R6-R9), story progression and save/load (R10-R12), Lua
stability (R13-R15) and measured performance (R16). Co-op (R24) is a
blocker for the co-op beta only.

### Build and test infrastructure

- **R1. Bring CI back** (SP; E2; T0). The self-hosted runner is offline
  since 2026-10-21 19:09 UTC and is installed as a console app, so it stops
  when its window or the PC session ends. Install it as a Windows service
  (the runner's own `config.cmd --runasservice`, or `svc.cmd install`) so
  it survives restarts; then re-run CI on `main`. *Needs the owner: it
  changes a service on the PC.* Done when: a CI run on `main` completes
  (pass or fail) without manual action after a reboot. Status: still open.
- **R2. Fix the platform-dependent userdata test** (SP; E2, E3; T0).
  `events.rs:1713` compares Lua's printed `%p` of a light userdata; it fails
  on i686 (`0000000C`) [measured] and Linux (`0xc`) [reported]. Assert the
  guid's value instead (for example `type(target)` and a number read back),
  and keep `tostring` of pointers out of assertions. One small PR. Done
  when: `cargo test -p m2-game` passes on x64 and i686. Status: still open.
- **R3. Re-run CI on every PR merged without it** (SP; E2; T0). #141's merge
  and #153, #157, #158, #159, #160, #162 landed with no CI result. Once R1
  and R2 are in, a green `main` run covers them; any further failure is a
  must-fix. Status: still open.
- **R4. Decide supported platforms** (SP; E3; T0). CI no longer builds
  Linux. Either add a Linux job (a GitHub-hosted runner costs Actions
  minutes the private repo no longer has) or state in the README that beta
  supports Windows x64 only. i686 stays tested because CI tests it. A
  decision, then a small PR. Status: undecided.

### Contracts in isolation (E5)

- **R5. Retarget PR #161 to `main`** (process). Its base `claude/polish` was
  merged by #159. `main` merges into it cleanly, so no rebase is needed.
  *Changing a PR's base is the owner's call or the polish thread's.*
  Status: still open.
- **R6. Merge or finish #152** (SP; E5; T6, T7). MecCon001 and JetCon001
  complete [PR], MecCon001 only with a race driver, which does not satisfy
  E5. Ready for review; waits on CI (R1). Status: fixed in an open PR.
- **R7. Race driving under autoplay** (SP; E5; T6). Six of the 13 missing
  contracts are races or race stages: PmcCon015 (never run), PmcCon016
  (7/27 gates), OilCon005 (timer out at gate 2), PirCon001 (jetski catches
  the pier at checkpoint 2), ChiCon008 (waits at an optional barrel instead
  of the race tank), AllCon002 (MLRS race won in some runs; boats stage
  lost). PmcCon003's race and MecCon001 rely on test drivers. The races
  thread owns this. Done when: each completes in 2 of 3 runs with no
  contract driver.
- **R8. The other seven missing contracts** (SP; E5; T6):
  - PmcCon013 (helicopter challenge) and PmcCon018 (airstrike challenge):
    playable, unlocked by the flow [code]; never run.
  - PmcCon032: paused with a local WIP fix (grenade launcher in the rifle
    slot); next step is the unrun round 45.
  - ChiCon002: 14 of 15 depot targets (bridge segments placed by #147).
  - ChiCon051, ChiCon053: not surveyed in any PR.
  - AllCon008: opens only after PmcCon004's consequence; test it in
    isolation with `--skip-to` and in the story run (R10).
- **R9. Make isolated completions repeatable** (SP; E5; T6). Run all 52 on
  one candidate commit, 3 runs each, no contract drivers (PmcCon004 needs
  its climb driver today; plain autoplay stalls in a gully 3.3 km out), and
  fill the T6 table. A contract below 2 of 3 is a must-fix.

### Story progression and save/load (E6, E7)

- **R10. Play the story without developer skip** (SP; E6, E7; T2b).
  Implement and run the T2b story test: a new game with skip mode off
  (`--movies`, no `--skip-to`), normal progression, save and reload at each
  defined boundary, through the final required story contract. Expect it
  to expose missing act-change behavior (consequences, HQ moves, act
  layers). Completions under `--skip-to` (R9) never count for this item.
- **R11. Faction HQs and act changes** (SP; E6; T2b). PMC HQ is proven;
  other HQs are reached only through `--skip-to` [beta checklist,
  2026-10-20; not re-checked]. Required for T2b to pass.
- **R12. Full start-up layer loading in free roam** (SP; E6; T2b, T3b).
  Free roam uses the start-up scripts' layer set only to place vehicles
  (`startup.rs`). Load the set's people, props and state layers too, per
  act, so free roam between contracts matches the game.

### Stability (E8a, E8b)

- **R13. Lua registry instrumentation, then the leak** (SP; E8a, E8b; T9).
  One harness panic since 2026-10-19: mlua "out of auxiliary stack space
  (used 7996 slots)". No fix in `main`'s history. First add a log line with
  the Lua registry size and live reference count at every `pace:` line and
  at each contract start/end and save/load, so growth is visible in T9 and
  T2b; then fix what it shows.
- **R14. `WifVzAtmosphere:79` concatenate-nil** (SP; E8b; T9). Reported by
  several threads as pre-existing. Trace which global is nil on that
  boundary (likely a stubbed or missing engine call) and fix.
- **R15. Detect and count hangs** (SP; E8a, E8b; T9). The harness counts
  panics, not hangs; a run killed at its limit looks the same as a stalled
  one. Add a watchdog log line (no frame or tick for 10 s), and keep the
  T9 ledger of cumulative test-hours and crashes.

### Performance (E9)

- **R16. Measure where play loses its frame rate** (SP; E9; T12). The
  latest human playtest ran at a median 25 fps with about 260 frames over
  50 ms a minute and three stalls over 1 s, while a frame draws in 7.9 ms
  at 720p (BETA_READINESS.md, "Performance baseline"); neither figure is
  controlled. First add per-part timings to the `pace:` line (tick,
  scripts, streaming, draw, present); then run the controlled T12
  benchmark (same hardware, 1920x1080, fixed settings, fixed route, warm-up,
  duration) on the current `main` and fix the largest measured cost. No
  optimization before that measurement.

### Release (E13)

- **R26. Release package check** (SP; E13; T14). Build the release archive
  the way the beta workflow does and run T14 on a clean Windows account.
  (Numbered with Tier 2's items so earlier references stay stable.)

## Tier 2: should fix during beta preparation

Important for a representative experience, but not blocking a playable,
testable single-player build. R24 is in this tier for the single-player
beta and becomes a blocker for the co-op beta.

- **R17. Satellite bomb lighting and fireball** (from the 360 code). With
  the bomb's own blast-script light the frame is darker (L 95 to 88 at
  45 m) because sunlit surfaces ignore `LightIntensity`; the fireball is
  about half the footage's size. Trace how the game's shaders apply
  `LightIntensity` before changing anything. Test: T5a.
- **R18. Particles inheriting effect movement.** `inheritvel` (and its
  spread) is parsed in `m2-assets` but not applied in the particle step.
  Trace the 360 particle spawn path for the velocity it inherits (the step
  itself already matches 2,000/2,000 in #161). Test: T5b.
- **R19. World wind and environmental forces.** The wind force kind exists
  in #161 but "takes the world's wind (the alpha has none)". Find the world
  wind source in the 360 code (the atmosphere or weather state) and its
  writers, then feed it to particles, flags and trees. Test: T5b.
- **R20. Explosions pushing nearby smoke.** Establish from the 360 code
  whether a blast adds a transient local force that other effects'
  particles feel (the five force kinds include an attractor and a vortex).
  Not yet traced. Test: T5b.
- **R21. Remaining world sound keys** (14 unresolved). Each is a hash with
  no cue; resolve through the cue lists and the 360 code's callers. Test:
  T5c.
- **R22. HUD edge rendering.** Open in the polish thread; measure against
  footage before fixing. Test: T3c.
- **R23. Jobs** (16 in the flow; 5 complete under autoplay, 11 in headless
  runs) [PR #150]. Not in the 52-contract definition, but a player meets
  them; each should be finishable by a human (T6c).
- **R24. Co-op to one shared contract** (**Co-op**; E11; T11). Stage 3 sits
  on `claude/coop-world` with no PR. Next: open its PR, then vehicles
  (physics needs a set-transform or a mirrored pose) and the mission view
  on the friend's screen, then one contract played together for 30
  minutes. **A blocker for the co-op beta; not part of the single-player
  beta,** which ships with co-op hidden or marked unsupported.
- **R25. Further performance fixes** (SP; E9; T12), after R16 has found and
  fixed the largest cost. Use the T12 benchmark and the particle-heavy
  scene (T5d); fix what the numbers show, not what looks slow.
- **R27. Rename the existing "Beta" release** (`beta-20261021`), which
  predates the beta definition, so testers aren't confused.
- **R28. Close superseded PRs #54 and #43** (owner's call).

## Tier 3: can wait until after beta

- R29. Grade: imageParams, saturation, villa overcast, island darkness, DOF,
  motion blur, bloom and flash brightness.
- R30. Unnamed data fields: vehicle physics floats, NodeHealth, WeaponHint
  words, FireAngle, `watr` header.
- R31. Vehicle gunners firing, bike records, crash damage to parts, handling
  from the template database instead of invented tuning.
- R32. Advanced video options, 4:3, slider mappings.
- R33. Android (#135 is on `main`; no CI) and the Linux build if R4 drops it.
- R34. Mod support (`m2-modapi` is empty).
- R35. Cut content (Osprey) and achievements.
- R36. The game's own save format (beta keeps the TOML save).
- R37. Co-op beyond one shared contract.

Visual polish, modding and reverse-engineering work that does not serve an
exit criterion waits here, so it does not take PC time from Tier 1.

## Criteria to items

| Criterion | Items | Tests |
|---|---|---|
| E1 Clean build | (passing) | T0 |
| E2 CI green | R1, R2, R3 | T0 |
| E3 Platform support | R2, R4 | T0 |
| E4 Start-up | - | T1 |
| E5 Contracts in isolation | R6, R7, R8, R9 | T6, T7 |
| E6 Story progression | R10, R11, R12 | T2b |
| E7 Save and load | R10 | T8, T2b |
| E8a Session stability | R13, R15 | T9 |
| E8b Aggregate crash rate | R13, R14, R15 | T9 ledger |
| E9 Performance | R16, R25 | T12, T9 |
| E10 Controls | - | T10 |
| E11 Co-op (co-op beta) | R24 | T11 |
| E12 Human playtest | all | T13 |
| E13 Release package | R26 | T14 |

## Ownership and concurrency

Threads already working on the PC (as of 2026-10-26): game polish (#161),
races, recruits (#152). Paused: PmcCon032 (local branch), PmcCon004 plain
autoplay. Keep to the PC's run rule (at most 3 game runs at once, low
priority). R1 to R4 touch only CI and one test and can run now without
disturbing them.

## How progress is reported

Each item's PR reports its before and after measurement. The T6 contract
table, the T2b story log and the release checklist in BETA_TEST_PLAN.md,
and the exit-criteria table in BETA_READINESS.md are where beta progress is
tracked; update them, don't append history.
