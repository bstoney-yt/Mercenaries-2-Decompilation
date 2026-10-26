# Beta readiness

Assessment of `main` at `de7ea9d` (2026-10-26), for the move from alpha to a
playable beta. Companion documents: [BETA_ROADMAP.md](BETA_ROADMAP.md) (what
to do, in order) and [BETA_TEST_PLAN.md](BETA_TEST_PLAN.md) (how to check it).

## Evidence labels

Every status below carries one of these:

- **[measured]**: measured for this assessment on `de7ea9d` (commands and
  numbers in "Baseline measurements").
- **[PR]**: claimed by a merged or open pull request's own before/after
  numbers. Usually one to three autoplay runs, often with `--skip-to`. Not
  re-run on current `main` for this assessment.
- **[code]**: read from the source on `main` (module docs, flags, tests).
- **[unverified]**: reported elsewhere (thread notes, older reports) or
  inferred; nobody has checked it on current `main`.

Provenance of game behavior keeps the project's own labels (from the 360
code, from samples, from footage, invented). A subsystem is not called
complete because its tests pass.

## The short answer

**The project is not close to beta. It is a capable alpha with a large,
mostly working mission layer, and its weakest points are reproducibility
and end-to-end play, not feature count.**

- **Beta definition (set 2026-10-23):** the build in which all 52 contracts
  in the mission flow complete, and which friends download for co-op.
- **Against that definition: 37 of 52 contracts have reached "contract
  completed" on `main` at least once [PR]; 2 more are in open PR #152 [PR];
  13 have never completed.** Of the 13, five are races or race-like stages
  (PmcCon016, OilCon005, PirCon001, ChiCon008, AllCon002), three are
  unsurveyed (PmcCon013/015/018), and the rest are blocked on one stall
  each (PmcCon032, ChiCon002, ChiCon051, ChiCon053, AllCon008).
- **Those 37 are not 37 reliable contracts.** Every count comes from
  `--skip-to` (the scripts' developer skip into one contract), mostly a
  single successful run, several with a contract-specific test driver
  (PmcCon003, PmcCon004, MecCon001) or a pass rate under 100 % (PmcCon034
  1 of 2, OilCon002 2 of 3). No PR or thread report describes a run that
  played the story from a new game past PmcCon001 in one save [unverified
  beyond that search].
- **`main` would fail its own CI today.** The `events.rs:1713` test fails on
  the i686 target, which CI's second job tests [measured]. It reached `main`
  with PR #141 at 19:03 UTC on 2026-10-21, minutes before the runner went
  offline, so no CI run has ever reported it.
- **The x64 build is green on this PC, but CI has not passed since 2026-10-21
  18:09 UTC** [measured]: the self-hosted runner has been offline since a
  job it was running stopped at 19:09 UTC that day, and 151 CI runs since
  2026-10-19 ended cancelled. Every PR merged after 2026-10-21 19:03 UTC landed
  without CI.
- **Play is too slow for beta.** The latest human playtest log shows a
  median 25 fps at 2560x1440 with frequent frames over 50 ms [measured from
  its log]. A headless 11-minute run showed no memory growth and no crash
  [measured].
- **No supported platform other than Windows x64 is built or tested
  automatically.** CI moved to a self-hosted Windows runner, so Linux is not
  built at all (the Linux failure in `events.rs` is real, but no CI job
  would catch it); Android has a build script but no CI.

A realistic estimate, given the pace of the last two weeks (about 4 to 8
contracts closed per working day across parallel threads, plus rework):
**the remaining 13 contracts are one to two weeks of focused work; making
all 52 pass reliably in a played-through save, with CI green, is the larger
part and is two to four weeks beyond that.** That estimate is a judgment,
not a measurement. No readiness percentage is given, because the gaps that
matter (end-to-end progression, repeatability) have never been measured.

## Exit criteria for beta

Each criterion is measurable and has a test in BETA_TEST_PLAN.md. "Now" is
the state on `de7ea9d`.

| # | Criterion | Measure | Now |
|---|---|---|---|
| E1 | Clean build | `cargo fmt --check`, `clippy --workspace --all-targets -D warnings`, `cargo test --workspace`, release build: all pass on Windows x64 | Pass [measured]: 1,064 tests, 0 failed |
| E2 | CI green | The CI workflow passes on `main` for the release commit, and on every PR merged in the 7 days before it | Fails: no CI pass since 2026-10-21 [measured] |
| E3 | Platform tests | Linux host `cargo test --workspace` passes (or Linux is formally dropped); the i686 build and `m2-game` tests pass | **Fails:** i686 `m2-game` 172 pass, 1 fail [measured]; Linux 1 failure, not in CI [reported] |
| E4 | Start-up | 20 of 20 cold launches reach the front end; new game reaches control of the hero; no launch over 60 s to the front end | Not measured as a series |
| E5 | Story completes | Each of the 52 flow contracts reaches `Pg.ContractCompleted` in at least 2 of 3 autoplay runs on the release commit, with no contract-specific driver | 37 at least once, mostly 1 run [PR]; 13 never |
| E6 | Story progression | One save carries the story from a new game through each act change (PmcCon003/004 consequences, HQ moves) to the last contract, by chaining runs through save and load | Never attempted |
| E7 | Save and load | A save after each act loads in a fresh launch with the same contracts done, the same active contract, and the hero where the save says | Verified for VzaCon001 only [PR #138] |
| E8 | Stability | No crash or hang in a 60-minute autoplay session; at most 1 crash per 10 hours across the release test series | Harness: 1 panic in 772 runs since 2026-10-19 [measured]; hangs not counted |
| E9 | Performance | At 1920x1080 on the test PC: median at least 60 fps in town and free roam; frames over 50 ms fewer than 60 per minute; memory growth under 10 % between minute 10 and minute 60 | **Fails** in the latest human playtest: median 25 fps, about 260 frames over 50 ms per minute at 2560x1440 [measured from its log]; memory flat over 11 min headless [measured] |
| E10 | Controls | Every action in the game's pad layout and the keyboard map works; pause menu, PDA and front end usable with pad, keyboard and mouse alone | Pad and keyboard maps on `main` [code]; no full checklist run |
| E11 | Co-op | Two PCs can host and join, see each other, and play one contract together for 30 minutes without desync of positions or a crash | Stage 1 only on `main` (see each other) [PR #145] |
| E12 | Human playtest | One complete human session per act with no blocker filed; every blocker found is fixed or waived in writing | Playtest 12 (2026-10-22) is the latest |

Beta does **not** require (can stay incomplete): faithful grade and bloom,
motion blur and depth of field, world wind, every unnamed data field,
achievements, the game's own save format, Android, mod support, cut
content (Osprey), and co-op beyond one shared contract.

## Subsystem audit

Line counts and test counts are for `main` at `de7ea9d` [measured]. The
provenance column counts doc-comment mentions in each crate's source
("360" = mentions the 360 code; "invented" and "inferred" as written), a
rough indicator, not a measure of correctness.

| Crate | Lines | Tests | 360 | invented | inferred |
|---|---|---|---|---|---|
| m2-alpha (the game) | 80,297 | 364 | 4 | 253 | 85 |
| m2-engine | 47,802 | 205 | 7 | 81 | 89 |
| m2-assets | 36,247 | 231 | 0 | 6 | 44 |
| m2-game (scripts, flow) | 22,560 | 173 | 1 | 8 | 36 |
| m2-render | 8,519 | 53 | 0 | 6 | 9 |
| m2-net (co-op) | 1,327 | 14 | 0 | 2 | 0 |
| m2-math, m2-sys, m2-hook, m2-layout | 1,948 | 26 | 0 | 0 | 0 |
| m2-modapi, m2-mem, m2-oracle, m2-loader | 38 | 0 | - | - | - |

Very little behavior on `main` is from the 360 code yet. PR #161 (open) is
the first large block of it (effect forces, particle step, spawning, glow
layer). Most gameplay tuning (handling, AI distances, autoplay rules) is
invented, which is acceptable for beta where the behavior is plausible and
labeled, and is the main source of "it doesn't feel like the game" reports.

### Asset loading (m2-assets)

- **Works:** WAD archives, models, textures, animation, terrain, layers,
  templates, Lua bytecode, voice, sound banks, Flash/GFx menus (an AVM1
  interpreter), effects [code, tests].
- **Partial:** several record fields unnamed (vehicle physics floats,
  NodeHealth, WeaponHint words, `watr` header) [scratch gap list].
- **Provenance:** formats from samples; names from the install's strings.
- **Tests:** 231, mostly round-trips and decodes of synthetic data; many
  depend on a local install for real-data checks.

### World, streaming and start-up layers (m2-engine `world`, `stream`; m2-alpha `startup`)

- **Works:** the island streams from `vz.wad`; buildings, props (70,000
  named), scatter and per-building collision [code].
- **Partial:** free roam runs the game's start-up scripts headless and uses
  the layers they load **for vehicles only** (`startup.rs`: "Free roam
  places the island's vehicles from that set"). People, props and state
  layers in free roam still come from all of `layers_static` or from the
  alpha's own spawns [code]. Jungle trees beside some roads are missing
  where the real game has them [unverified, free-roam round 19 report].
- **Missing:** full start-up layer loading in free roam (people, props,
  state layers per act).
- **Depends on:** act changes and HQ moves (state layers per act).

### Scripts and mission flow (m2-game)

- **Works:** the game's own Lua scripts run (`--mission`); objectives,
  markers, briefings (258 clips), subtitles, contract consequences,
  `WifMissionFlow`, save data handed back on load [PR #130, #138].
- **Partial:** 37 of 52 contracts completed once [PR]; jobs: 16 start,
  5 complete under autoplay, 11 complete in headless runs [PR #150].
- **Known bug:** `WifVzAtmosphere:79` concatenates nil on a boundary
  (pre-existing, reported by several threads) [unverified cause].
- **Known bug:** `events.rs:1713` test compares Lua's `tostring` of a light
  userdata, whose `%p` text is platform-specific (`000000000000000C` on
  MSVC x64, `0000000C` on MSVC i686, `0xc` on glibc). Fails on i686
  [measured: 172 pass, 1 fail] and on Linux [reported in the 2026-10-26
  weekly report]. Fix: assert the guid value, not its printed form.
- **Tests:** 173.

### Missions and autoplay (m2-alpha `mission`, `autoplay`)

- **Works:** autoplay drives most contract types: outposts, deliveries,
  escorts, HVT verify, bombing, sniping, emplaced guns, hijacks [PR].
- **Partial:** races (boat, jetski, car, tank) are the main remaining
  family; PmcCon004 completes only with its climb driver; PmcCon003's race
  stage uses a test driver [PR #141, #160].
- **Invented:** nearly all autoplay rules and distances (labeled in PRs).
  Autoplay is a test tool, not game behavior, so this is acceptable.
- **Risk:** `--skip-to` uses the scripts' developer skip mode. Completion
  under skip does not prove the contract is reachable in a played story.

### Player, combat and weapons (m2-alpha `player`, `combat`, `hijack`; m2-engine `weapons`)

- **Works:** on foot, swim, rifle/RPG/grenades, soldiers' own guns, sniper
  scope and aim zoom, damage falloff, hijacks with the game's prompts, pad
  vibration [PR #136, #159].
- **Invented:** most timings and damage values outside the data's
  `DamageDropoff` and weapon templates.
- **Missing:** vehicle gunners firing (no NPC-crewed fire) [gap list].

### Vehicles (m2-engine `physics`, `vehicle`, `vehicle_damage`)

- **Works:** cars, trucks, tanks, boats, helicopters drive; wrecks;
  brake/reverse lamps [PR #159]; tank crush [branch].
- **Partial:** crash damage to parts, bike records unridden, engine and air
  drag, brakes outliers [gap list]. Handling is invented on Rapier.
- **Blocks:** race contracts depend on driving well enough under autoplay.

### Effects, explosions and physics (m2-engine `particles`; PR #161)

- **On `main`:** effect shapes, particles leaving along the emitter's
  normal [PR #159].
- **In PR #161 (open):** the five force kinds, the 360 particle step
  (2,000/2,000 samples match), spawning by the 360 curve clock, glow layer,
  sprite sizes, the satellite bomb's own blast script light [PR, from the
  360 code].
- **Missing (named in the brief):**
  - World wind: a wind force "takes the world's wind (the alpha has none,
    so a wind does nothing)" [code, #161 `particles.rs`].
  - Particles inheriting effect movement: `inheritvel` is parsed in
    `m2-assets` but not used by the particle step [code, #161].
  - Explosions pushing nearby smoke: no blast force applied to other
    effects' particles [code search, unverified in play].
  - Satellite bomb: frame darker with the script light (L 95 to 88 at
    45 m) because sunlit surfaces ignore `LightIntensity`; fireball about
    half the footage's size [thread state, measured by that thread].

### Rendering (m2-render)

- **Works:** wgpu (D3D12/Vulkan) backend, skinning, instancing, shadows,
  post, gamma-correct HUD blend, windowed/borderless/fullscreen [PR #143,
  #146].
- **Partial:** grade (saturation, overcast, island darkness), bloom and
  flash brightness, HUD edge rendering [thread state].
- **Missing:** motion blur, depth of field (constants set by the program,
  not the data) [gap list].

### Audio (m2-engine `audio`; m2-alpha `sound`)

- **Works:** 3D effects, engines, ambience, music, radios, pause hold,
  briefings and VO, cue-list fallbacks [PR #156, #158].
- **Missing:** 14 world sound keys unresolved [thread state]; loudness
  levelling; no audio in automated runs, so sound regressions are caught
  only by human playtests.

### HUD, menus and front end (m2-alpha `gamehud`, `ui`, `shell`, `pause`; m2-game `gui_runtime`)

- **Works:** the game's own Flash HUD and menus, front end with saves,
  Continue/Load, options kept, attract video, Quit to main menu [PR #139,
  #155, #156].
- **Partial:** PDA (stand-in screens remain in `ui.rs`), some advanced
  video options shown but unused, HUD edges.

### Input (m2-alpha `input`, `pad`)

- **Works:** XInput pads with the game's own layout from `tControlMap`
  [code]; keyboard and mouse with PC button pictures [PR #161 branch].
- **Open PRs:** #54 (Xbox controller) and #43 (Android) are stale drafts,
  480 and 642 commits behind `main`. #54's change has an equivalent on
  `main` (`git cherry` marks it applied); #43 was superseded by #135
  (merged). Both can be closed.

### Saves (m2-game `savedata`; m2-alpha)

- **Works:** own TOML save, scripts' save data restored, hero placed where
  the save says [PR #138]. Autosave exists [branch `claude/shop-money-autosave`].
- **Unverified:** saves across act changes, HQ moves and after PmcCon004's
  consequences.

### Co-op (m2-net)

- **On `main`:** stage 1, host or join, see the other hero; a join page;
  a beta release workflow (tag `beta-20261021`) [PR #145].
- **On branch `claude/coop-world` (no PR):** stage 3, people shared both
  ways, the friend's hits sent to the host.
- **Missing:** vehicles, missions on the friend's screen, contract sync.
  Co-op is the furthest subsystem from the beta definition.
- **Note:** a GitHub release is already named "Beta" (`beta-20261021`).
  That name predates the beta definition; rename it to avoid confusion.

### Modding (m2-modapi)

- **Missing entirely:** the crate is 2 lines of doc comment [measured].
  Out of beta scope; should be stated in the README.

### Parked crates

`m2-loader`, `m2-hook`, `m2-oracle`, `m2-mem` are placeholders from the
abandoned hook-DLL plan (see `docs/alpha-roadmap.md`). No beta impact.

## Open pull requests (2026-10-26)

| PR | Branch → base | State | Finding |
|---|---|---|---|
| #152 Recruits (MecCon001, JetCon001) | `claude/recruits` → `main` | ready for review, 8 ahead, 0 behind, merges cleanly | MecCon001 uses a race test driver. CI queued, cannot run (runner offline). |
| #161 Polish 2 (explosions, forces, glow, bomb) | `claude/polish-2` → `claude/polish` | draft, 12 ahead, 10 behind `main` | **Needs retargeting to `main`.** Its base `claude/polish` is fully merged (PR #159, merge `2d400cf`), so merging #161 as is would land in a dead branch. A merge of `main` into it is clean (`git merge-tree`), so no rebase is needed, only the base change. |
| #54 Xbox controller | → `claude/button-glyphs` | draft, 480 behind | Superseded; close. |
| #43 Android | → `claude/project-thread-puymzh` | draft, 642 behind | Superseded by #135; close. |

## Baseline measurements

All on `main` `de7ea9d`, in a fresh worktree, on the test PC (Windows 11,
RTX 4080), with other project threads running at the same time.

| Check | Result |
|---|---|
| `cargo fmt --all --check` | pass (2 s) |
| `cargo clippy --workspace --all-targets` with `-D warnings` | pass (84 s, incremental deps) |
| `cargo test --workspace --no-fail-fast` | pass: 1,064 tests, 0 failed, 0 ignored (153 s) |
| `cargo xtask symbols` | pass |
| `cargo build --release -p m2-alpha` | pass (220 s) |
| `cargo test -p m2-game --target i686-pc-windows-msvc` (a CI step) | **fail**: 172 pass, 1 fail (`events.rs:1713`, left `userdata: 0000000C`, right `userdata: 000000000000000C`) |
| CI history (last 200 runs of the CI workflow) | 35 success (2026-10-20 to 21), 12 failure, 151 cancelled; last `main` success 2026-10-21 18:09 UTC (`f95f05f`) |
| Self-hosted runner | offline; its log ends mid-job at 2026-10-21 19:09 UTC; installed as a console app, not a service, so it does not restart with the PC |
| Panics in harness runs since 2026-10-19 | 1 in 772 runs (`stderr` logs): mlua "cannot create a Lua reference, out of auxiliary stack space" (ChiCon050, 2026-10-19); no fix found in `main`'s log, so treat as open |

## Performance baseline

Two sources. Neither is a controlled benchmark: other project threads were
building and running at the same time, and the game window was not in front.

**A. Headless autoplay run made for this assessment** (`de7ea9d`,
`--mission --autoplay --mission-seconds 1500 --after-contract 900`, 1280x720,
idle priority, sampled every 15 s):

| Measure | Result |
|---|---|
| New game to VzaCon001 `Pg.ContractCompleted` | completed; then 900 s of free roam; clean exit after 663 s wall time |
| Loading | island tiles ready after 2.8 s; props within reach after 13.3 s |
| Memory | working set 2.5-2.8 GB, private 5.1-5.3 GB; 5,121 MB at 136 s, 5,337 MB peak at 437 s, 5,207 MB at 648 s: no growth trend over 11 min |
| CPU | about 1 core busy throughout (CPU seconds equal wall seconds) |
| GPU | 1-10 % busy, 4.2 GB of video memory (headless; not representative of play) |
| Draw cost at the last position | 7.89 ms a frame (127 fps): tick 0.10, streaming 0.60, draws 2.23, submit 4.97 ms (60 frames, render only) |
| Crashes | none |

The harness simulates without drawing every frame, so it cannot give the
frame rate in play. That needs a windowed run (`pace:` lines).

**B. The latest human playtest log** (playtest 12, 2026-10-22, build of
`main` then, 2560x1440, 31 `pace:` spans of 30 s, about 15 minutes):

| Measure | Result |
|---|---|
| Frame rate per span | min 4, median 25, mean 29, max 60 fps; 17 of 31 spans under 30 fps |
| Frames over 50 ms | 4,016 in 15.5 min (about 260 a minute) |
| Stalls | 3 spans with a single frame over 1 s (worst 4,321 ms) |

So play today runs at about half the beta target and stutters, while a
single frame draws in under 8 ms at 720p. The gap points at per-frame work
outside drawing (simulation, scripts, streaming hitches, or the window's
present path) rather than GPU load. That is a hypothesis to measure (see
roadmap item 16), not a finding.

**Not yet measured:** a particle-heavy scene, a 60-minute session, 1080p
windowed play on the current `main`, and load time to the front end.
