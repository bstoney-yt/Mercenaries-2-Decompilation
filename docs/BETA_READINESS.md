# Beta readiness

Assessment of `main` at `de7ea9d` (2026-10-26), for the move from alpha to a
playable beta. Revised the same day: release targets split into
single-player and co-op beta, story progression separated from isolated
contract tests, the three unsurveyed PMC ids settled from the scripts, and
every finding rechecked against the repository (see "Findings status").

Companion documents: [BETA_ROADMAP.md](BETA_ROADMAP.md) (what to do, in
order; items are numbered R1, R2, ...) and
[BETA_TEST_PLAN.md](BETA_TEST_PLAN.md) (how to check it; tests are numbered
T0, T1, ...). Each exit criterion below names its roadmap items and tests.

## Evidence labels

Every status below carries one of these:

- **[measured]**: measured for this assessment on `de7ea9d` (commands and
  numbers in "Baseline measurements").
- **[PR]**: claimed by a merged or open pull request's own before/after
  numbers. Usually one to three autoplay runs, often with `--skip-to`. Not
  re-run on current `main` for this assessment. A PR claim does not prove
  the change is on `main`; merged PRs are on `main`, open PRs are not.
- **[code]**: read from the source on `main` (module docs, flags, tests) or
  from the game's own scripts in the install.
- **[unverified]**: reported elsewhere (thread notes, older reports) or
  inferred; nobody has checked it on current `main`.

Provenance of game behavior keeps the project's own labels (from the 360
code, from samples, from footage, invented). A subsystem is not called
complete because its tests pass.

## Release targets

The beta definition set on 2026-10-23 was: the build in which all 52
contracts in the mission flow complete, and which friends download for
co-op. That definition is kept, as the **co-op beta**. Because co-op is the
subsystem furthest from done, this revision adds a smaller, explicit
**single-player beta** so a playable build does not wait on networking.

| Target | Required criteria | Explicitly excluded |
|---|---|---|
| **Single-player beta** | E1-E10, E12 (single-player sessions), E13 | Co-op of any kind (E11). The release notes and README say so; the join page and co-op menu entries are hidden or marked unsupported. |
| **Co-op beta** (the 2026-10-23 definition) | Everything in the single-player beta, plus E11, plus a co-op session in E12 | Nothing beyond the "can wait" list below. |

E11 is not weakened: the co-op beta needs it in full.

**Decided (2026-10-26):** the single-player beta ships first, with co-op
hidden or clearly marked unsupported; the co-op beta is a later target that
must satisfy every single-player criterion plus E11. The supported platform
for the beta is Windows only (E3); Linux builds stay best-effort developer
builds until Linux CI and clean-package testing are reliable. The CI runner
runs as a Windows service with automatic start and recovery, under a
low-privilege account, with a read-only job token and no code from forks'
pull requests. The roadmap and
test plan mark each item and test as **SP** (needed by both targets) or
**Co-op** (needed by the co-op beta only).

## The short answer

**The project is not close to either beta. It is a capable alpha with a
large, mostly working mission layer, and its weakest points are
reproducibility and end-to-end play, not feature count.**

- **Contracts: 37 of 52 have reached "contract completed" on `main` at least
  once [PR]; 2 more are in open PR #152 [PR]; 13 have never completed.**
  All 52 are real contracts: each has its own script module in `vz.wad`,
  including PmcCon013, PmcCon015 and PmcCon018 [code; see below]. Of the 13,
  six are races or race stages (PmcCon015, PmcCon016, OilCon005, PirCon001,
  ChiCon008, AllCon002), two are challenges never run (PmcCon013,
  PmcCon018), and five are blocked on one stall each (PmcCon032, ChiCon002,
  ChiCon051, ChiCon053, AllCon008).
- **Against the normal-play criterion the count is lower.** Contract
  drivers do not count, so PmcCon003 and PmcCon004 (and MecCon001 in #152)
  are not passes: **35 of 52 driver-free on `main`, 36 with #152.** None of
  them has a recorded 3-run pass rate on one commit; the historical "at
  least once" status is no longer the measure (see BETA_TEST_PLAN.md T6).
- **No contract has been completed in normal story progression.** Every
  completion used `--skip-to` (the scripts' developer skip into one
  contract), and a headless `--mission` run without `--movies` keeps the
  scripts' skip mode on for the whole session [code: `mission.rs`
  `with_skip_mode_off`; PR #158]. The VzaCon001 completion measured for
  this assessment ran that way, so it does not count either. No PR or
  thread report describes a story run past PmcCon001 in one save
  [unverified beyond that search].
- **`main` would fail its own CI today.** The `events.rs:1713` test fails on
  the i686 target, which CI's second job tests [measured]. It reached `main`
  with PR #141 at 19:03 UTC on 2026-10-21, minutes before the runner went
  offline, so no CI run has ever reported it.
- **CI has not passed since 2026-10-21 18:09 UTC** [measured]: the
  self-hosted runner has been offline since a job it was running stopped
  at 19:09 UTC that day, and 151 CI runs since 2026-10-19 ended cancelled.
  Every PR merged after 2026-10-21 19:03 UTC landed without CI.
- **Play is too slow for beta.** The latest human playtest log shows a
  median 25 fps at 2560x1440 with frequent frames over 50 ms [measured from
  its log]. That is not a controlled benchmark; a 1080p benchmark on a fixed
  route comes first (R16, T12).
- **Only Windows x64 is built and tested automatically.** Linux is not built
  anywhere (CI moved to a self-hosted Windows runner); Android has a build
  script but no CI. The beta is Windows-only (decided 2026-10-26); Linux
  stays a best-effort developer build (R4).

A realistic estimate, given the pace of the last two weeks (about 4 to 8
contracts closed per working day across parallel threads, plus rework):
**the remaining 13 contracts are one to two weeks of focused work; making
all 52 pass 2 of 3 without drivers, and then in one played-through story
without developer skip, with CI green and performance measured, is the
larger part and is two to four weeks beyond that, for the single-player
beta.** The co-op beta adds co-op stages 3 onward, which have no estimate
because no shared contract has been attempted. These are judgments, not
measurements. No readiness percentage is given, because the gaps that
matter (story progression, repeatability, performance under control) have
never been measured.

## Exit criteria

Each criterion is measurable and has a test in BETA_TEST_PLAN.md. "Now" is
the state on `de7ea9d`. Every criterion ends a release candidate as
**pass**, **fail** or **waived** (a written waiver with reason and owner),
recorded in the test plan's release checklist.

| # | Target | Criterion | Measure | Tests | Roadmap | Now |
|---|---|---|---|---|---|---|
| E1 | SP | Clean build | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo xtask symbols`, release build: all pass on Windows x64 | T0 | - | Pass [measured]: 1,064 tests, 0 failed |
| E2 | SP | CI green | The CI workflow passes on `main` for the release commit, and on every CI-applicable, code-changing PR merged in the 7 days before it. A docs-only PR that the workflow's path filter skips is recorded as "skipped (paths)", not as a failure | T0 | R1, R3 | **Fail**: no CI pass since 2026-10-21 [measured] |
| E3 | SP | Platform support | Supported platform: Windows (decided 2026-10-26). The Windows x64 and i686 builds and the CI's tests pass; Linux is best-effort and not a criterion | T0 | R2, R4 | **Fail on `main`**: i686 `m2-game` 172 pass, 1 fail [measured]; fixed in open PR #164 (173 pass on x64 and i686) [measured] |
| E4 | SP | Start-up | 20 of 20 cold launches reach the front end in under 60 s; new game reaches control of the hero | T1 | - | Not measured as a series |
| E5 | SP | Contracts in isolation | Each of the 52 flow contracts reaches `Pg.ContractCompleted` in at least 2 of 3 autoplay runs with `--skip-to` on the release commit, with no contract-specific driver | T6 | R6-R9 | 35 driver-free at least once [PR]; 0 with a 3-run record on one commit; 13 never |
| E6 | SP | Story progression | One playthrough from a new game with developer skip off follows normal progression to B8, the final story contract PmcCon004 and its `WifMissionFlow` consequence (ending movie, credits, return to the front end): contracts appear when the flow unlocks them, consequences apply, act changes and HQ moves happen, through save and reload at each boundary (T2b) | T2b | R10-R12 | **Never attempted** |
| E7 | SP | Save and load | At every T2b boundary, a save loads in a fresh launch with the same contracts done, the same active and available contracts, the same act, and the hero where the save says | T8, T2b | R10 | Verified for VzaCon001 only [PR #138] |
| E8a | SP | Session stability | A single 60-minute session (contract, then free roam) with no crash and no hang (no frame for 10 s) | T9 | R13-R15 | Not run; hangs not detectable yet |
| E8b | SP | Aggregate stability | At most 1 failure (a crash or a hang, counted together) per 10 test-hours across the release test series, with at least 20 cumulative test-hours on the candidate | T9 (ledger) | R13-R15 | Harness: 1 panic in 772 runs since 2026-10-19 [measured]; hangs not detected; test-hours not tracked |
| E9 | SP | Performance | Controlled benchmark (T12: fixed hardware, 1920x1080, fixed settings, fixed route, 60 s warm-up, 10 min measured): median at least 60 fps; fewer than 60 frames over 50 ms per minute; memory growth under 10 % between minute 10 and minute 60 of T9 | T12, T9 | R16, R25 | **Fail** in the latest human playtest (uncontrolled): median 25 fps at 2560x1440 [measured from its log]; memory flat over 11 min headless [measured] |
| E10 | SP | Controls | Every action in the keyboard/mouse map and the pad layout works in each context (on foot, vehicle, aircraft, boat, menus, PDA) per the T10 checklist | T10 | - | Maps on `main` [code]; no checklist run |
| E11 | Co-op | Co-op | Two PCs host and join, see each other, and play one contract together for 30 minutes without position desync or a crash | T11 | R24 | Stage 1 only on `main` (see each other) [PR #145] |
| E12 | SP (+ Co-op) | Human playtest | One complete human session per act (and, for co-op beta, one co-op session) using the T13 report template; every blocker fixed or waived | T13 | - | Playtest 12 (2026-10-22) is the latest; no template |
| E13 | SP | Release package | The release archive, on a clean Windows account with no repository or tools, finds the install, starts, writes its settings and a save, keeps settings across restarts, and shuts down cleanly | T14 | R26 | Not tested |

Beta does **not** require (can stay incomplete): faithful grade and bloom,
motion blur and depth of field, world wind, every unnamed data field,
achievements, the game's own save format, Android, mod support, cut
content (Osprey), and co-op beyond one shared contract. Visual polish,
modding and research that does not serve an exit criterion should not take
PC time from the criteria above.

## Contract accounting

The 52 ids are the contracts `WifMissionFlow` binds or unlocks [code:
`--example wad -- scripts` on `vz.wad`; flow listing]. Each one's name hash
matches a script module in `vz.wad` (52 of 52; `m2_assets::name_hash`,
checked for this revision). The three previously unsurveyed ids:

| Id | Unlocked by | Its script defines | Kind (inferred from the names) |
|---|---|---|---|
| PmcCon013 | `UnlockMission("PmcCon013")` in the UP introduction's flow step, beside OilCon050 and PmcCon034 | `Start`, `Complete`, `CopterDestroyed`, `PollHeight`, `PmcCon013_VehicleObjective`, `StartTheShallenge` | Helicopter challenge |
| PmcCon015 | `UnlockMission("PmcCon015")` behind `HasKey("PmcCon016")` | `StartRace`, `CourseUnfinished`, `VehicleDeath`, `LoadAssets` | Race |
| PmcCon018 | `UnlockMission("PmcCon018")` when Misha joins (after JetCon001) | `Complete`, `AirStrikes`, `AirStrikeVO`, `CurPoints`, `CoinToss` | Airstrike challenge |

All three are playable contracts, so the definition stays at 52.

**The story's end, and what it removes** [code: `WifMissionFlow`, the
`PmcCon004` binding, script lines 605-716]. PmcCon004 is the final story
contract: its prerequisite is `HasKey("PmcCon004")`, and its consequence
(`fConseq`) verifies Solano, plays the ending movie (`15_ACK_`) and the
credits, forces an autosave, then requests the `unloading` game state and
quits to the front end. The same consequence calls `DestroyMission` on
AllCon001, AllCon002, AllCon003, AllCon008, AllCon050, ChiCon001, ChiCon002,
ChiCon003, ChiCon008, ChiCon050 and GurCon003, and destroys the Allied and
China starters. Two consequences:

- Those 11 contracts must be completed **before** PmcCon004 in a story run,
  or they are gone. E5 still tests them in isolation.
- AllCon008 is unlocked (after AllCon052) **only while PmcCon004 is not
  done** (`HasKey("PmcCon004")` guards the `UnlockMission`). The earlier
  note that it "opens only after PmcCon004" (from the 2026-10-20 beta
  checklist) was wrong and is corrected here.

The
per-contract table (passes out of three, driver, skip, story run,
outstanding failure) is kept in BETA_TEST_PLAN.md, T6.

## Findings status

Rechecked on 2026-10-26 at 12:55 UTC: `origin/main` is still `de7ea9d`; open
PR heads unchanged (#152 `5f6f101`, #161 `8914cca`, #54 `d69a1a1`, #43
`3d94b37`); runner still offline.

| Finding | Status |
|---|---|
| CI runner offline, console app not a service | Still open (install script prepared; needs the owner) |
| `events.rs:1713` userdata test fails on i686 and Linux | Fixed in an open PR (#164) |
| No CI pass on `main` since 2026-10-21 | Still open |
| #161 targets the merged `claude/polish`; needs retargeting to `main` (clean merge, no rebase) | Fixed: retargeted 2026-10-26; checks in progress |
| AllCon008 "opens only after PmcCon004" | Superseded: the flow unlocks it only before PmcCon004 [code] |
| #54 and #43 superseded | Superseded (by `main` and #135); still open as PRs |
| 37 contracts completed once on `main` | Needs remeasurement (T6 on one commit) |
| MecCon001 and JetCon001 complete | Fixed in an open PR (#152) |
| Effect forces, particle step, glow layer, bomb blast light | Fixed in an open PR (#161) |
| World wind, `inheritvel`, smoke pushed by blasts, fireball size, bomb lighting | Still open (also absent from #161) |
| PmcCon013/015/018 unsurveyed | Resolved: playable contracts (above) |
| Lua auxiliary stack panic (one, 2026-10-19) | Needs remeasurement (T9 with registry logging) |
| `WifVzAtmosphere:79` concatenate-nil | Still open [unverified cause] |
| Playtest 12 frame rate (median 25 fps) | Needs remeasurement (T12, controlled) |
| Free roam uses start-up layers for vehicles only | Still open [code] |
| Saves across act changes | Needs remeasurement (never measured) |
| "Beta" release `beta-20261021` predates the beta definition | Still open |

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
  state layers per act) (R12).
- **Depends on:** act changes and HQ moves (state layers per act).
- **Tests:** human check T3b.

### Scripts and mission flow (m2-game)

- **Works:** the game's own Lua scripts run (`--mission`); objectives,
  markers, briefings (258 clips), subtitles, contract consequences,
  `WifMissionFlow`, save data handed back on load [PR #130, #138].
- **Partial:** 37 of 52 contracts completed once [PR]; jobs: 16 start,
  5 complete under autoplay, 11 complete in headless runs [PR #150].
- **Known bug:** `WifVzAtmosphere:79` concatenates nil on a boundary
  (pre-existing, reported by several threads) [unverified cause] (R14).
- **Known bug:** `events.rs:1713` test compares Lua's `tostring` of a light
  userdata, whose `%p` text is platform-specific (`000000000000000C` on
  MSVC x64, `0000000C` on MSVC i686, `0xc` on glibc). Fails on i686
  [measured: 172 pass, 1 fail] and on Linux [reported in the 2026-10-26
  weekly report]. Fix: assert the guid value, not its printed form (R2).
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
  under skip does not prove the contract is reachable in a played story,
  and is never counted for E6.

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
- **In PR #161 (open, not on `main`):** the five force kinds, the 360
  particle step (2,000/2,000 samples match), spawning by the 360 curve
  clock, glow layer, sprite sizes, the satellite bomb's own blast script
  light [PR, from the 360 code].
- **Missing:**
  - World wind: a wind force "takes the world's wind (the alpha has none,
    so a wind does nothing)" [code, #161 `particles.rs`] (R19).
  - Particles inheriting effect movement: `inheritvel` is parsed in
    `m2-assets` but not used by the particle step [code, #161] (R18).
  - Explosions pushing nearby smoke: no blast force applied to other
    effects' particles [code search, unverified in play] (R20).
  - Satellite bomb: frame darker with the script light (L 95 to 88 at
    45 m) because sunlit surfaces ignore `LightIntensity`; fireball about
    half the footage's size [thread state, measured by that thread] (R17).

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
- **Missing:** 14 world sound keys unresolved [thread state] (R21);
  loudness levelling; no audio in automated runs, so sound regressions are
  caught only by human playtests (T5c).

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
- **Tests:** none end to end; T10 is the checklist.

### Saves (m2-game `savedata`; m2-alpha)

- **Works:** own TOML save, scripts' save data restored, hero placed where
  the save says [PR #138]. Autosave exists [branch `claude/shop-money-autosave`].
- **Unverified:** saves across act changes, HQ moves and after PmcCon004's
  consequences (T2b, T8).

### Co-op (m2-net) — co-op beta only

- **On `main`:** stage 1, host or join, see the other hero; a join page;
  a beta release workflow (tag `beta-20261021`) [PR #145].
- **On branch `claude/coop-world` (no PR):** stage 3, people shared both
  ways, the friend's hits sent to the host.
- **Missing:** vehicles, missions on the friend's screen, contract sync.
  Co-op is the furthest subsystem from the co-op beta.
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
| #152 Recruits (MecCon001, JetCon001) | `claude/recruits` → `main` | ready for review, 8 ahead, 0 behind, merges cleanly | MecCon001 uses a race test driver (not a normal-play pass). CI queued, cannot run (runner offline). |
| #161 Polish 2 (explosions, forces, glow, bomb) | `claude/polish-2` → `main` (retargeted 2026-10-26; was `claude/polish`) | draft, 12 ahead, 10 behind `main` | **Retargeted.** Diff against `main` unchanged: 12 commits, 28 files, +3,268 -417. Before: Its base `claude/polish` is fully merged (PR #159, merge `2d400cf`), so merging #161 as is would land in a dead branch. A merge of `main` into it is clean (`git merge-tree`), so no rebase is needed, only the base change. |
| #54 Xbox controller | → `claude/button-glyphs` | draft, 480 behind | Superseded; close. |
| #43 Android | → `claude/project-thread-puymzh` | draft, 642 behind | Superseded by #135; close. |
| #163 This assessment | `claude/beta-plan` → `main` | draft | Docs only. |
| #164 CI fixes | `claude/userdata-test` → `main` | draft | R2 test fix and CI hardening (R1). |

## Baseline measurements

All on `main` `de7ea9d`, in a fresh worktree, on the test PC (Windows 11,
RTX 4080), with other project threads running at the same time.

| Check | Result |
|---|---|
| `cargo fmt --all --check` | pass (2 s) |
| `cargo clippy --workspace --all-targets` with `RUSTFLAGS=-D warnings` (as CI sets it; equivalent to `-- -D warnings` for this workspace) | pass (84 s, incremental deps) |
| `cargo test --workspace --no-fail-fast` | pass: 1,064 tests, 0 failed, 0 ignored (153 s) |
| `cargo xtask symbols` | pass |
| `cargo build --release -p m2-alpha` | pass (220 s) |
| `cargo test -p m2-game --target i686-pc-windows-msvc` (a CI step) | **fail**: 172 pass, 1 fail (`events.rs:1713`, left `userdata: 0000000C`, right `userdata: 000000000000000C`) |
| CI history (last 200 runs of the CI workflow) | 35 success (2026-10-20 to 21), 12 failure, 151 cancelled; last `main` success 2026-10-21 18:09 UTC (`f95f05f`) |
| Self-hosted runner | offline; its log ends mid-job at 2026-10-21 19:09 UTC; installed as a console app, not a service, so it does not restart with the PC |
| Panics in harness runs since 2026-10-19 | 1 in 772 runs (`stderr` logs): mlua "cannot create a Lua reference, out of auxiliary stack space" (ChiCon050, 2026-10-19); no fix found in `main`'s log, so treat as open |

## Performance baseline

Two sources. Neither is a controlled benchmark: other project threads were
building and running at the same time, and the game window was not in
front. They locate the problem; they do not size it. T12 is the controlled
benchmark that E9 is judged on.

**A. Headless autoplay run made for this assessment** (`de7ea9d`,
`--mission --autoplay --mission-seconds 1500 --after-contract 900`, 1280x720,
idle priority, sampled every 15 s; scripts' skip mode on, so not a story
run):

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
| Where each second went (mean of the 31 spans' `pace:` breakdown; spans nest, so parts overlap) | ticks 424 ms (game 212, scripts 203, of which Lua 118; player and physics 97; people 66; crews aboard 64), present 351 ms, draw 166 ms, stream 12 ms |

So play then ran at about half the beta target and stuttered, while a
single frame draws in under 8 ms at 720p. The breakdown (on `main` since
`18503c1`) shows two large costs: the fixed-step simulation (ticks) and
present, which at 2560x1440 likely includes waiting on the GPU. That is
uncontrolled evidence; R16 measures it on the T12 route at 1080p before
any optimization.

**Not yet measured:** the controlled 1080p benchmark (T12), a
particle-heavy scene, a 60-minute session, and load time to the front end.

## Decisions

Taken 2026-10-26: single-player beta first; Windows-only beta; CI runner as
a low-privilege Windows service with automatic start and recovery; #161
retargeted to `main` (done, 13:3x UTC; diff unchanged at 12 commits, 28
files).

Still open: closing #54 and #43; renaming the `beta-20261021` release; the
repository's fork-PR approval policy (see below).

**Security note.** The repository is public (`visibility: public`), and its
fork pull-request approval policy is `first_time_contributors`, so a
contributor's second fork PR would run without approval on the self-hosted
runner. PR #164 makes the CI jobs skip pull requests whose branch is not in
this repository and sets the job token to read-only. Setting the policy to
"all outside collaborators" in the repository's Actions settings closes it
at the source as well (an owner's setting).
