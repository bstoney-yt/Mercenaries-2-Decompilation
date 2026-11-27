# Feature checklist

What the single-player game has, and how much of it the port has. Built
on 2026-11-26 against `main` at `02aa7c7e`.

**Where the feature list comes from.** Public descriptions of the game:
the GamePressure guide (index, tips, outposts, additional targets,
challenges, spare parts pages), the Wikipedia article's gameplay section,
and a few press previews. They are summarized here in our own words.
The guides are not complete. Each feature should be checked against the
game's own scripts and templates before work on it starts.

**What the status means.**

- **done**: the code implements it end to end, and a run, a test or a
  check has shown it working. This is not the same as faithful: open
  fidelity gaps are linked.
- **partial**: it runs, but part of it is invented, missing or never
  completed in a recorded run.
- **missing**: no code for it.
- **unknown**: the sources disagree, or we have not checked the game's
  data for it yet.

The evidence is the code on `main` (module docs, tests), the nightly
autoplay night `20261126-0605Z` (18 contracts run, 3 passed), the
known-bugs issue #698, [GAPS.md](fidelity/GAPS.md) and the
[contract tracker](contract-tracker.md). Nothing here was checked by
playing the game.

Most of the game's rules live in its Lua scripts (factions, shops,
outposts, HVTs, jobs, contracts), and the port runs those scripts. So for
those features the port's job is the engine services the scripts call,
and a feature counts as done when the scripts get what they ask for.

## Heroes and on-foot play

| Feature | Status | Evidence and what is left |
|---|---|---|
| Three heroes (Mattias, Jennifer, Chris), each with own health, model, clips and voice | done | `hero.rs`; chosen in the shell (`flow.rs`) |
| Hero upgrades and costumes (wardrobe at HQ) | partial | `Player.SetOutfit` and profile upgrade fields read (`game.rs`, `mission.rs`); per-upgrade stat effects not audited |
| Walk, run, sprint, crouch, jump | partial | Speeds fitted to clips, instant starts and stops, full air control: GAPS M1, M2, A5 |
| Animation transitions and layering | partial | Blend times from the 360 (`blend.rs`), state table read (`human_states.rs`); time scale, partition mask and gait selection still ours: GAPS A2 to A4 |
| Swimming | partial | Runs; depths, speeds and stroke rate invented: GAPS M4 |
| Ladders | done | `ladders.rs`, `m2-engine/ladder.rs` |
| Third-person camera on foot | partial | Distances invented (GAPS M3); pad turn curve differs (#690, #692) |
| Health, regeneration, survival mode under 10 health | done | #695 merged |
| Death: contract cancelled, wake up in the sickbay | done | #632 merged |
| Knockdown, stagger, ragdoll from blasts | partial | Ragdoll rounds 1 to 4 merged (#699, #707, #710, #722, #767); joint motors, limp camera and fall damage in open PRs (#721, #730, #737, #738) |
| Fall damage | done | #618 merged; limp-body falls in #730 |
| Melee, including bashing | partial | Shove strength and blow size invented (#698) |
| Grappling hook (helicopter hijack) | done | `grapple.rs` |
| Binoculars | partial | Camera from the 360 in open #724 |
| Parachute | unknown | No code; check whether the game has one before treating it as missing |

## Weapons and support

| Feature | Status | Evidence and what is left |
|---|---|---|
| Rifles, launchers, grenades, C4, mines | done | `combat.rs`, `mines.rs` (#634) |
| Ammunition pickups and munitions laptops | done | #660, #755 |
| Homing missiles and locks | partial | Lock shape and proximity burst invented (#698) |
| Mounted guns and vehicle gunner seats | partial | Seat cameras in open #705, #713, #718 |
| Projectile flight (gravity, noise) | partial | Gravity merged (#714, #715); FlightNoise in open #719; guidance law: GAPS P6 |
| Explosions: damage, push, rays | partial | Push on vehicles from the 360 (#728, #768); rays for all targets in open #758, #761 |
| Support menu: buy in shops, call in with designators | done | `support.rs`, `--shop-check` covers the whole catalogue |
| Air strikes, satellite bomb, bunker buster | done | `strikes.rs`, `m2-game/airstrike.rs` |
| Helicopter deliveries (crates, vehicles) | partial | Flight path, heights and hover time invented (`flights.rs`) |
| Air defenses (SAM, AA) blocking support | done | Markers and script rules run (AllCon003, #688) |

## Vehicles

| Feature | Status | Evidence and what is left |
|---|---|---|
| Cars, trucks, jeeps: driving | partial | Tyres, springs, coast from the 360; engine, gears and parts of the response still on Rapier's controller with invented shapes: GAPS P1 |
| Tanks | partial | Track drive and braking invented (`physics.rs`) |
| Motorbikes | partial | Balance from the 360; fallback invented |
| Boats | partial | Float and boat sim from the 360; wreck buoyancy invented |
| Helicopters (player-flown) | partial | `_HelicopterPhysics` step from the 360; cameras in open #706, #711 |
| Hijacking, including tank and helicopter sequences | done | `hijack.rs`, `ridehijack.rs` |
| Vehicle disguise | partial | Rules on main; how fast it drops and recovers invented (#698) |
| Vehicle damage, wrecks, sinking | done | #667, #673 |
| Crash damage to parts | partial | Tier 3 item R31 |
| Car radio music | unknown | Outpost radios play (`music_source.rs`); in-car radio not checked |

## Factions and relations

| Feature | Status | Evidence and what is left |
|---|---|---|
| Six factions (VZ, UP, PLAV, China, Allied Nations, Pirates) | done | Templates and relations read (`m2-engine/ai.rs`, `api.rs`) |
| Standing from friendly to hostile, set by the scripts | done | Relations -100 to 100 kept for the scripts (`api.rs`) |
| Infractions: witnesses report her, standing drops | partial | `Ai.AddInfraction` and the pursuit clock run; who sees and reports is not audited against the 360 |
| Pursuits and reinforcements | done | `pursuers.rs`, `reinforce.rs` from the 360 |
| Bribes through Fiona | partial | The briefing's bribe option is wired; not run end to end in a recorded check |
| Faction gates opening for friends, territory | partial | Gate rule and honk window in `api.rs`; territory is the scripts' |
| Soldier AI: planner, senses, cover, targeting | partial | Goal planner, senses and cover from the 360; Anchor component missing: GAPS I1 |

## Economy and the PMC

| Feature | Status | Evidence and what is left |
|---|---|---|
| Cash and fuel, pickups, payouts | done | Pickups in play; kept in the save |
| Penalty for killing civilians | unknown | Scripts' rule; not checked |
| Faction shops, stock growing with contracts | done | `--shop-check` |
| PMC HQ and its moves | partial | PMC HQ proven; HQ moves only reached with `--skip-to` (BETA_ROADMAP R11) |
| Recruits: Ewan (pilot), Eva (mechanic), Misha (jet pilot) | partial | Recruit contracts run; MecCon001 and JetCon001 fail in the nightly |
| Ewan: fly her out, pick-ups at captured outposts | partial | Pause menu's fly-out wired; outpost pick-ups not checked |
| Spare parts for Eva | unknown | A guide lists 100 to collect; no code reference; check the scripts |

## Story and side content

| Feature | Status | Evidence and what is left |
|---|---|---|
| 52 story contracts | partial | Nightly 2026-11-26: 18 run, 3 pass (VzaCon001, PmcCon002, OilCon003); most failures are the test bot stuck, not yet split into bot or game faults |
| Story played through without developer skip | missing | Never attempted (BETA_READINESS E6) |
| Acts, map opening up, consequences | partial | Scripts run; act changes not proven in one save |
| Ending: final contract, movie, credits | unknown | Never reached |
| Outposts: take them, troops flown in | partial | Scripts and test-bot support exist; not counted |
| HVTs: verify (photo) or capture (stow and extract) | partial | Verify photo and StowHVT action exist; capture flight not checked |
| Additional targets: buildings and billboards | partial | `m2-game/jobs.rs` handles bounty and collection jobs |
| Jobs and challenges (races, deliveries, Pirate rum runs) | partial | 16 jobs in the flow, about 5 complete under autoplay (R23) |
| Races | partial | Six race contracts have never completed (R7) |
| Wagers | partial | Wager box and change-wager dialog run (`gamehud.rs`); payout not checked |
| Destructible buildings and props | partial | `destruct.rs`, `breakables.rs`; whole-object rule merged (#623); props as bodies ongoing (GAPS P7) |
| Ambient life: traffic, civilians, benches | done | `traffic.rs`, `social.rs` |

## Front end, HUD and saves

| Feature | Status | Evidence and what is left |
|---|---|---|
| Boot movies, shell, new game, load | done | `flow.rs`, `shell.rs` |
| Cut-scenes and lip-sync | partial | Played; hero lines use mouth flapping (#674) |
| HUD, PDA map, contract screens | partial | Flash renderer lacks gradients, filters and blend modes: GAPS U1, U2, U4, U6 |
| Pause menu and options | done | `pause.rs` |
| Saves in the 360's record layout, autosave | done | #781, #782, #784 |
| Achievements | partial | Granted by the scripts, shown in the launcher (#786); no platform service |
| Subtitles | done | `subtitles.rs` |
| Sound: weapons, vehicles, impacts, music | partial | Collision and bullet-hit sounds in open PRs (#684, #689, #694, #696) |

## Out of scope for now

Co-op (two heroes visible only), day and night (the game has none),
motion blur, performance work, ray tracing and mod support are parked
(BETA_ROADMAP Tier 3 and the project's settled direction).

## The largest gaps

These are the biggest missing or invented systems that block or shape
single-player play, by size of the remaining port:

1. **Wheeled and tracked vehicle drive** (GAPS P1). Engine, gearbox,
   clutch and the tank's tracks are still an invented model over the
   templates' fields. Races and chases depend on it.
2. **Human locomotion** (GAPS M1, M2, A2 to A5). Starting, stopping, air
   control, gait speeds and clip rates are ours. Every moment on foot
   depends on it.
3. **Flash renderer** (GAPS U1, U2, U4, U6). Gradients, filters, blend
   modes and text layout, used by every HUD and menu screen.

Separately, story progression in one save (E6) has never been tried. It
is a test, not a port, but it is the measure of a finished game.

## Sources

- GamePressure guide: https://www.gamepressure.com/mercenaries2worldinflames/game-guide/z38c71
- Wikipedia: https://en.wikipedia.org/wiki/Mercenaries_2:_World_in_Flames
- Den of Geek review: https://www.denofgeek.com/?p=157255
- GameSpot preview: https://gamespot.com/articles/mercenaries-2-world-in-flames-updated-look/1100-6173344/
