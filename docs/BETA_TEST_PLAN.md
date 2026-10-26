# Beta test plan

Repeatable checks for the exit criteria in [BETA_READINESS.md](BETA_READINESS.md).
Each test says whether it is automated (autoplay or a harness), needs a
human, or both. Automated runs follow the PC's run rule: at most three game
runs at once, each at low priority. All game data is read from the
player's own install at run time; nothing from the game is copied into the
repository or attached to reports.

`<install>` is the game's install folder. `m2-alpha` writes its log to
stderr, so the evidence lines below are grepped from the run's `.err` file.

## 0. Build gate (every candidate)

Automated. Run in a fresh worktree of the candidate commit.

```
cargo fmt --all --check
cargo clippy --workspace --all-targets            # RUSTFLAGS=-D warnings
cargo test --workspace --no-fail-fast
cargo test -p m2-game --target i686-pc-windows-msvc
cargo xtask symbols
cargo build --release -p m2-alpha
```

Pass: all exit 0, and the CI workflow is green on the same commit (E1,
E2, E3). Record test totals.

## 1. Fresh installation and first launch

- **Automated (E4):** 20 cold launches with `--to-front-end`, killed 60 s
  after the front end appears. Evidence: the front end's first-frame line
  and launch-to-front-end seconds. Pass: 20 of 20, all under 60 s.
- **Human:** unzip the release into an empty folder on a PC that has never
  run it; launch with no arguments (the install is found automatically);
  check the attract video, the menus with mouse, keyboard and pad, and
  that options are kept after a restart. Pass: no step needs a workaround.

## 2. New game and initial progression

- **Automated (E5, E6):** `--mission --autoplay --mission-seconds 1500
  --after-contract 300` from a new game with skip mode off. Evidence:
  `Pg.ContractCompleted` for VzaCon001, then PmcCon001 offered. Pass:
  2 of 3 runs.
- **Human:** play VzaCon001 and PmcCon001 by hand. Check briefings,
  subtitles keeping up with voice, objectives and markers, the villa.

## 3. Free-roam exploration

- **Automated (E8, E9):** after VzaCon001, free roam for 15 minutes with
  autoplay driving between points across the island. Evidence: `pace:`
  lines every 30 s (fps, slowest frame, frames over 50 ms), memory samples
  every 15 s, no panic. Pass: E9's thresholds.
- **Human (visual):** drive the coast road, the jungle and Caracas at day
  and night. Compare against the reference footage on the PC (frames stay
  on the PC): vehicles and people where the game has them, trees beside
  roads, lighting and grade. File each difference with a frame number.

## 4. Vehicle acquisition and destruction

- **Automated:** the outpost and hijack contracts (GurCon050, PmcCon004)
  exercise boarding and hijacks. Pass: completion as in test 6.
- **Human:** get into each class (car, truck, tank, boat, helicopter),
  hijack a tank and a helicopter with the pad and with the keyboard, call
  a vehicle delivery, wreck one of each class. Check brake and reverse
  lamps, engine sounds, the wreck's fire and smoke.

## 5. Weapons and explosions

- **Automated:** the satellite bomb test with `M2_SAT_AHEAD=<m>` at 45 m
  and 120 m; measure luminance and fireball size on the screenshots with
  the polish thread's scripts. Pass: within the bounds set from the
  footage in that thread's report.
- **Human:** every hand weapon, grenades, C4, an airstrike, a bunker
  buster, the satellite bomb. Check smoke near blasts, particles following
  moving effects, glow, and sound at range. **Particle-heavy scene for
  performance:** three airstrikes in Caracas within 10 s; log `pace:` and
  memory (E9).

## 6. Contract acceptance and completion

- **Automated (E5):** all 52 flow contracts, 3 runs each, with
  `--mission --autoplay --mission-seconds 1500 --skip-to <id>` and no
  `--mission-driver`. Evidence: `Pg.ContractCompleted` and the seconds it
  took. Keep the result table below up to date (pass count per contract).
  Pass: every contract 2 of 3 or better.
- **Human:** accept a contract from the HQ menu and from a contact in the
  field; check the PDA map, tracking, the pay, and that a failed contract
  can be retried.

### Contract result table

Status on `main` `de7ea9d` from PR evidence (not re-run). `1+` = completed
in at least one run; `d` = needed a contract-specific test driver.

| Faction | Completed at least once | Not yet |
|---|---|---|
| Story | VzaCon001, PmcCon001, PmcCon002, PmcCon003 (d), PmcCon004 (d), PmcCon031, PmcCon033, PmcCon034 (1 of 2) | PmcCon016, PmcCon032, PmcCon013, PmcCon015, PmcCon018 |
| UP | OilCon001, 002 (2 of 3), 003, 020, 021, 050, 051, 052 | OilCon005 |
| Guerrillas | GurCon001, 002, 003, 005, 050, 052, 053 | |
| Pirates | PirCon002, 003, 004, 051, 052 | PirCon001 |
| Allied | AllCon001, 003, 050, 052, 053 | AllCon002, AllCon008 |
| China | ChiCon001, 003, 009, 050 | ChiCon002, 008, 051, 053 |
| Recruits (PR #152) | MecCon001 (d), JetCon001 | |

Totals: 37 on `main`, 2 in #152, 13 not yet, 52 in the flow.

## 7. Recruit progression

- **Automated:** MecCon001 and JetCon001 (`-skip` each); evidence: the
  recruit flag (`mechanic false -> true`) and the `MecPmcBoss` /
  `JetPmcBoss` starters. Pass: 2 of 3 each, no driver.
- **Human:** after each recruit joins, check what it unlocks (the
  mechanic's vehicles, the pilot's deliveries) in the shop and the PDA.

## 8. Save, quit and reload

- **Automated (E7):** after each act's last contract, `--save-to`, then a
  fresh launch with `--load`. Evidence: the scripts' save line (`N done
  [...], active [...]`) identical before and after, and the hero within
  2 m of the saved place. Pass: identical for every act.
- **Human:** save from the pause menu, Quit to main menu, Continue; quit
  the game during an autosave; load an older save after a newer one.

## 9. Extended play sessions

- **Automated (E8, E9):** a 60-minute autoplay session (a contract, then
  free roam) with memory sampled every 15 s. Pass: no panic or hang,
  memory growth under 10 % between minute 10 and minute 60, `pace:` within
  E9. Also the target test for the Lua reference leak (roadmap item 13):
  log the Lua registry size at each `pace:` line if available.
- **Human:** a 2-hour session by a tester, noting any slowdown, audio
  dropouts or visual corruption with the time it happened.

## 10. Co-op, where supported

- **Automated:** `coop_peer` (`--host` / `--join`, `--people N`) against
  the alpha with `--coop-seconds 600`. Evidence: both peers log each
  other's positions; no protocol errors. Pass: 10 minutes with no desync
  warning.
- **Human (needs two PCs):** host and join through the join page; play
  one contract together for 30 minutes (E11). Never paste an invite code
  in a public place; it carries the host's public address.

## Human versus automated

Autoplay can prove that scripts reach completion, that the game doesn't
crash or leak, and give frame-rate and memory numbers. It cannot judge:
how the game looks against footage, how it sounds (test runs start no
audio), how controls feel, whether a human can finish races and timed
stages, menus by hand, and co-op between two people. Those need test 1-5,
7, 8 and 10's human parts, run once per release candidate, with each
finding filed as a numbered blocker or waived in writing (E12).

## Release candidate checklist

1. Build gate (test 0) green, CI green.
2. Contract table: 52 rows at 2 of 3 or better (or the definition amended
   for non-playable flow ids).
3. Story chain through save and load (test 8) passes.
4. 60-minute session (test 9) passes.
5. Human playtest, one session per act, no open blockers.
6. Co-op test 10, or beta declared single-player.
