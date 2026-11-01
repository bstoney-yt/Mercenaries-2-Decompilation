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
| **Recovered** | Read directly from the original. Always say which kind: **recovered data** (a structure and its values, decoded from the game's files) or **recovered code** (a consumer traced in the 360 program: what the runtime does with the data) | From samples / From the 360 code |
| **Verified** | Our Rust behavior matches a reference derived from recovered code (an emulator of the traced 360 function) on stated test cases, within a stated tolerance | From the 360 code, plus the test that pins it |
| **Inferred** | Follows from several pieces of evidence, but nobody has read it directly | Inferred |
| **Approximated** | The evidence is incomplete, so values were fitted to footage or measurements | Inferred (fitted) |
| **Invented** | The original behavior is unknown, so we chose values that work | Invented |

When the original behavior itself has not been established, the gap
record says **Unknown** and names the research needed. Unknown describes
the original. The five levels describe what we know about it.

### What each step does and does not establish

Recovery happens in separate steps. Each step proves only its own claim:

1. **Data recovered.** We can parse a table, read its rows and name its
   columns. This shows the structure and values exist. It does not show
   how the runtime uses them. Example: decoding the transition table's
   rows is recovered data.
2. **Consumer traced.** We have found the 360 function that reads the data
   and recovered what it computes: selection rules, equations, order,
   defaults, how special values like -1 are handled. Only this step shows
   how the original engine uses the data. Example: knowing how the engine
   picks and applies a transition means tracing the code that reads the
   transition table.
3. **Reimplemented.** Our Rust code does what the traced consumer does.
   This step is still unverified.
4. **Verified.** A separate check compares our Rust against a reference
   built from step 2, for example an emulator of the traced function, and
   reports the error.

Some evidence can look stronger than it is:

- **Field names found by hash match.** The database keys fields by hash.
  A name that hashes to a key (`MaxForce`, `GearOptRPM`) is a strong
  match, but its meaning is still inferred until a consumer is traced.
- **Agreement between two decoders.** If our decoder agrees with an
  independent decoder, both read the data the same way. That is not
  verification against the original runtime.
- **Footage.** Footage is behavioral or visual evidence of what the player
  saw. It can support Inferred or Approximated, and it can check a visible
  result. It is never proof of the original's internal implementation.
- **Working in play.** "It looks right" or "the contract completes" shows
  the behavior is plausible. It does not show the behavior is recovered.

Matching an emulator of the 360 code earns "Verified". It does not prove
hardware equivalence (VMX128 rounding, denormals, operation order on the
real CPU), and the record should say which reference and tolerance were
used.

## Gap record

Each gap in [GAPS.md](GAPS.md) has an id (`A` animation, `M` character
movement, `P` physics, `U` UI), a class, a priority and these eight fields:

1. **Original**: what the original game does, with its evidence level
   (or Unknown), naming which step above each claim has reached.
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
