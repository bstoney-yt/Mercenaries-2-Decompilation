# Fidelity program: physics, animation, character movement and UI

Standing engineering direction (owner, 2026-10-26): reproduce the original
game's physics, animation, character movement and UI behavior as faithfully
as the evidence allows. This work comes **after** the current beta priorities
(CI restore and the 32-bit test fix, the no-skip story test, timing
instrumentation and the 1080p benchmark).

The menus and most character animations already work and are not to be
rewritten. Treat them as working implementations that need targeted
corrections. Fix shared root causes before per-item patches. Accept a
per-item exception only when the original data or code proves it.

## Files

| File | What it holds |
|---|---|
| [ROADMAP.md](ROADMAP.md) | Subsystem inventory (what exists, what is known, evidence, gaps, validation, complexity, dependencies) and the recommended order |
| [GAPS.md](GAPS.md) | The fidelity-gap tracker: one record per gap, prioritized |

The audit behind both files was made from the code on `main` at `de7ea9d`
and the project's notes. It was done without the PC: nothing here was
checked against footage, the game's data or the 360 program in this round.
Every gap says which of those checks it still needs.

## Evidence levels

Every recovered behavior carries one of these labels. They extend the
module-doc provenance tags in the project instructions (From the 360 code,
From samples, From footage, Invented).

| Level | Meaning | Module-doc tag |
|---|---|---|
| **Recovered** | Read directly from the original code or data | From the 360 code / From samples |
| **Verified** | Matches an original-derived reference (an emulator of the 360 code, decoded data) numerically | From the 360 code, plus the test that pins it |
| **Inferred** | Follows from several pieces of evidence but was not read directly | Inferred |
| **Approximated** | Evidence is incomplete; values fitted to footage or measurements | Inferred (fitted) |
| **Invented** | Original behavior unknown; chosen to work | Invented |

Matching an emulator of the 360 code is "Verified", not proof of hardware
equivalence. Say which reference and tolerance were used.

## Gap record

Each gap in [GAPS.md](GAPS.md) has an id (`A` animation, `M` character
movement, `P` physics, `U` UI), a class, a priority and these eight fields:

1. **Original**: what the original game does (with its evidence level).
2. **Ours**: what the Rust implementation does now (file and line).
3. **Difference**: what a player sees.
4. **Likely cause**: the underlying cause, shared or local.
5. **Evidence available**: data, 360 code, footage, measurements.
6. **Proposed correction**.
7. **Verification**: the measurement before and after, and the regression
   test that pins it.
8. **Risk**: what else could break, and how to guard it.

**Classes:** Functional bug · Visual discrepancy · Timing discrepancy ·
Animation/movement discrepancy · Behavioral discrepancy · Unknown pending
investigation.

**Priority** is visible impact times how often players meet it:

- **P1**: seen constantly (every step, every drive, every menu screen).
- **P2**: seen often (every fight, every vehicle type, every mission).
- **P3**: seen occasionally or only when looking for it.

Do not polish P3 gaps while P1 gaps in the same system are open.

## Rules for fixing a gap

- One measurable outcome per round: measure, change, measure again, report
  both numbers.
- Research before a change: find the original behavior in the 360 program
  or the data first. A footage fit is "Approximated" and says so in code.
- Shared code (the `Animator`, the character controller, the vehicle model,
  the Flash renderer) changes only with a regression check across the whole
  roster it serves: every hero and soldier rig, every vehicle class, every
  menu movie, every supported aspect ratio.
- Keep working behavior. Where a new rule disagrees with a pick that was
  matched to footage, log the disagreement and resolve it with evidence
  rather than silently replacing the pick.
- Clean-room rules are unchanged: never commit game assets, executable
  bytes, decompiler databases or raw decompiler output; record addresses,
  names, constants and equations as permitted symbol metadata
  (`symbols/*.toml`) and in module docs.

## Updating the tracker

Add a gap when a playtest, footage comparison or code read finds one. Close
it with the PR that fixes it, the before/after numbers and the test name.
Keep closed gaps in a short "Closed" table at the end of GAPS.md so the
evidence stays findable.
