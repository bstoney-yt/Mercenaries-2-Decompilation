# Beta roadmap

The prioritized plan from [BETA_READINESS.md](BETA_READINESS.md) (state of
`main` at `de7ea9d`, 2026-10-26). Each item names its exit criterion (E1 to
E12), the measurement that closes it, and its evidence. Items are meant to
become one PR each, in the established worktree and PR workflow.

Order within a tier is the suggested order. Tier 1 items 1 to 4 come first
because every later item's "done" depends on a build that CI can check.

## Tier 1: must fix before beta

These prevent reliable building, testing, progression or play.

### Build and test infrastructure

1. **Bring CI back** (E2). The self-hosted runner is offline since
   2026-10-21 19:09 UTC and is installed as a console app, so it stops
   when its window or the PC session ends. Install it as a Windows service
   (the runner's own `config.cmd --runasservice`, or `svc.cmd install`) so
   it survives restarts; then re-run CI on `main`. *Needs the owner: it
   changes a service on the PC.* Done when: a CI run on `main` completes
   (pass or fail) without manual action after a reboot.
2. **Fix the platform-dependent userdata test** (E2, E3). `events.rs:1713`
   compares Lua's printed `%p` of a light userdata; it fails on i686
   (`0000000C`) [measured] and Linux (`0xc`) [reported]. Assert the guid's
   value instead (for example `type(target)` and a number read back), and
   keep `tostring` out of assertions. One small PR. Done when: `cargo test
   -p m2-game` passes on x64 and i686.
3. **Re-run CI on every PR merged without it** (E2). #153, #157, #158,
   #159, #160, #162 and #141's merge landed with no CI result. Once 1 and 2
   are in, a green `main` run covers them; any further failure is a
   must-fix.
4. **Decide Linux** (E3). CI no longer builds Linux. Either add a Linux job
   (a GitHub-hosted runner costs Actions minutes the private repo no longer
   has) or state in the README that beta supports Windows x64 only. A
   decision, then a small PR.

### Story and progression

5. **Retarget PR #161 to `main`** (process). Its base `claude/polish` was
   merged by #159. `main` merges into it cleanly, so no rebase is needed.
   *Changing a PR's base is the owner's call or the polish thread's.*
6. **Merge or finish #152** (E5). MecCon001 and JetCon001 complete [PR],
   MecCon001 with a race driver. Ready for review; waits on CI (item 1).
7. **Race driving under autoplay** (E5). Five of the 13 missing contracts
   are races or race stages: PmcCon016 (7/27 gates), OilCon005 (timer out
   at gate 2), PirCon001 (jetski catches the pier at checkpoint 2),
   ChiCon008 (waits at an optional barrel instead of the race tank),
   AllCon002 (MLRS race won in some runs; boats stage lost). PmcCon003's
   race and MecCon001 rely on test drivers. The races thread owns this.
   Done when: each completes in 2 of 3 runs with no contract driver.
8. **The other eight missing contracts** (E5):
   - PmcCon032: paused with a local WIP fix (grenade launcher in the rifle
     slot); next step is the unrun round 45.
   - ChiCon002: 14 of 15 depot targets (bridge segments placed by #147).
   - ChiCon051, ChiCon053: not surveyed in any PR.
   - AllCon008: opens only after PmcCon004's consequence; needs a
     progression run, not `--skip-to`.
   - PmcCon013, PmcCon015, PmcCon018: named in the flow but with no
     binding of their own; first establish from the scripts whether they
     are playable contracts or flow markers. If they are not playable, the
     beta count becomes 49 and the definition should say so.
9. **Make completions repeatable** (E5). Re-run all 52 on the release
   candidate, 3 runs each, no contract drivers (PmcCon004 currently needs
   its climb driver; its plain autoplay stalls in a gully 3.3 km out).
   Record pass rate per contract in a table kept with the test plan. A
   contract below 2 of 3 is a must-fix.
10. **Play the story in one save** (E6, E7). Chain contracts through save
    and load with skip mode off: new game, VzaCon001, PmcCon001 to 004
    (the act change and nuke consequences that remove AllCon001/002/003/
    008/050), HQ moves, through to the last contract. Every contract today
    is proven only in isolation through the developer skip. Expect this to
    expose missing act-change behavior (beta checklist: "Act changes and
    HQ moves across the whole story", "Faction HQs as hubs").
11. **Faction HQs and act changes** (E6). PMC HQ is proven; other HQs are
    reached only through `--skip-to` [beta checklist, 2026-10-20; not
    re-checked]. Required for a played-through story.
12. **Full start-up layer loading in free roam** (E6). Free roam uses the
    start-up scripts' layer set only to place vehicles (`startup.rs`).
    Load the set's people, props and state layers too, per act, so free
    roam between contracts matches the game.

### Stability

13. **Lua reference exhaustion** (E8). One harness panic since 2026-10-19:
    mlua "out of auxiliary stack space (used 7996 slots)". No fix in
    `main`'s history. Find the leaking references (a long session will
    show it: see BETA_TEST_PLAN.md, test 9) and fix.
14. **`WifVzAtmosphere:79` concatenate-nil** (E8). Reported by several
    threads as pre-existing. Trace which global is nil on that boundary
    (likely a stubbed or missing engine call) and fix.
15. **Measure hangs** (E8). The harness counts panics, not hangs; a run
    killed at its limit looks the same as a stalled one. Add a watchdog log
    line (no frame for N seconds) so hangs are countable.

### Performance

16. **Find where play loses its frame rate** (E9). The latest human
    playtest ran at a median 25 fps with about 260 frames over 50 ms a
    minute and three stalls over 1 s, while a frame draws in 7.9 ms at
    720p (BETA_READINESS.md, "Performance baseline"). First add per-part
    timings to the `pace:` line (tick, scripts, streaming, draw, present),
    then run a windowed 1080p session on the current `main` and fix the
    largest measured cost. No optimization before that measurement.

## Tier 2: should fix during beta preparation

Important for a representative experience, but not blocking a playable,
testable build.

17. **Satellite bomb lighting and fireball** (from the 360 code). With the
    bomb's own blast-script light the frame is darker (L 95 to 88 at 45 m)
    because sunlit surfaces ignore `LightIntensity`; the fireball is about
    half the footage's size. Trace how the game's shaders apply
    `LightIntensity` before changing anything.
18. **Particles inheriting effect movement.** `inheritvel` (and its
    spread) is parsed in `m2-assets` but not applied in the particle step.
    Trace the 360 particle spawn path for the velocity it inherits (the
    step itself already matches 2,000/2,000).
19. **World wind and environmental forces.** The wind force kind exists in
    #161 but "takes the world's wind (the alpha has none)". Find the
    world wind source in the 360 code (the atmosphere or weather state)
    and its writers, then feed it to particles, flags and trees.
20. **Explosions pushing nearby smoke.** Establish from the 360 code
    whether a blast adds a transient local force that other effects'
    particles feel (the five force kinds include an attractor and a
    vortex). Not yet traced.
21. **Remaining world sound keys** (14 unresolved). Each is a hash with no
    cue; resolve through the cue lists and the 360 code's callers.
22. **HUD edge rendering.** Open in the polish thread; measure against
    footage before fixing.
23. **Jobs** (16 in the flow; 5 complete under autoplay, 11 in headless
    runs) [PR #150]. Not in the 52-contract definition, but a player meets
    them; at least each should be finishable by a human.
24. **Co-op stage 3 to a shared contract** (E11). Stage 3 sits on
    `claude/coop-world` with no PR. Next: vehicles (physics needs a set-
    transform or a mirrored pose) and the mission view on the friend's
    screen. Co-op is part of the beta definition (the build friends
    download), so the minimum is: host and friend finish one contract
    together. If that is not reachable, ship beta as single-player and say
    so.
25. **Further performance fixes** (E9), after item 16 has found and
    fixed the largest cost. Use the baseline in
    BETA_READINESS.md and the particle-heavy scene in the test plan; fix
    what the numbers show, not what looks slow.
26. **Rename the existing "Beta" release** (`beta-20261021`), which
    predates the beta definition, so testers aren't confused.
27. **Close superseded PRs #54 and #43** (owner's call).

## Tier 3: can wait until after beta

28. Grade: imageParams, saturation, villa overcast, island darkness, DOF,
    motion blur, bloom and flash brightness.
29. Unnamed data fields: vehicle physics floats, NodeHealth, WeaponHint
    words, FireAngle, `watr` header.
30. Vehicle gunners firing, bike records, crash damage to parts, handling
    from the template database instead of invented tuning.
31. Advanced video options, 4:3, slider mappings.
32. Android (#135 is on `main`; no CI) and the Linux build if item 4 drops
    it.
33. Mod support (`m2-modapi` is empty).
34. Cut content (Osprey) and achievements.
35. The game's own save format (beta keeps the TOML save).

## Ownership and concurrency

Threads already working on the PC (as of 2026-10-26): game polish (#161),
races, recruits (#152). Paused: PmcCon032 (local branch), PmcCon004 plain
autoplay. Keep to the PC's run rule (at most 3 game runs at once, low
priority). Tier 1 items 1 to 4 touch only CI and one test and can run now
without disturbing them.

## How progress is reported

Each item's PR reports its before and after measurement. The contract
table (item 9) and the exit criteria table in BETA_READINESS.md are the
two places beta progress is tracked; update them, don't append history.
