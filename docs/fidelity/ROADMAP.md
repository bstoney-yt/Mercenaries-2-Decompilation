# Fidelity roadmap

How to move physics, animation, character movement and UI toward the
original, in order. Gap ids refer to [GAPS.md](GAPS.md). State of `main` at
`de7ea9d` (2026-10-26).

**When.** None of this starts before the current beta priorities are done:
(1) CI restored and the 32-bit test fixed, (2) the no-skip story test,
(3) timing instrumentation and the 1080p benchmark. Stage F0 below changes
no gameplay, but it needs the game's data on the PC, so it waits too
unless the owner says otherwise.

## Findings in brief

- **Animation (A1 to A4).** Most of the "repeated attempts" trace to one
  root: the runtime plays clips well, but it does not read the game's
  animation logic. Transitions use one fixed 0.25 s linear crossfade;
  playback rate uses an invented 0.6 to 1.4 clamp; the body split for aiming
  is derived from the skeleton; gait clips are found three different ways.
  The game's own tables carry the missing data and are already in the
  archives: the transition table (`ab8fe34b`), the state graph
  (`ece70371`), and the `MinTimeScale`, `MaxTimeScale`, `PartitionMask`,
  `ActionMask` and `LocomotionMask` columns. The clip decoder itself is
  sound (all 4,232 clips, within 7.6e-6 of an independent decoder).
- **Character movement (M1, M2).** The controller is a conventional
  kinematic capsule: instant starts and stops, full air control, invented
  step, slope, jump and turn values. This feeds foot sliding as much as
  the animation side does.
- **Physics (P1, P2).** Vehicles read the templates' rich handling fields
  (engine curve, gears, clutch, per-axle suspension, friction, brakes) but
  run them through Rapier's ray-cast vehicle with invented response
  shapes. Explosions push with a fixed invented velocity; the explosive's
  `MaxForce` is read and unused.
- **UI (U1, U2).** The menus are the game's own Flash movies and scripts,
  so behavior and navigation are largely original. The remaining
  differences concentrate in the shared Flash renderer (gradients, filters,
  blend modes) and in prompts drawn in our own font.

## Subsystem inventory

Complexity: S (a round or two), M (several rounds), L (a thread's week),
XL (multi-thread or multi-week).

### Animation playback and blending (A1, A2, A4)

| | |
|---|---|
| **Exists** | `m2_engine::Animator`: one clip with a crossfade, an optional upper-body clip, event cues, root pinning (`crates/m2-engine/src/anim.rs`) |
| **Known about the original** | Clips are Havok 5.5 (recovered). Key-driven selection through action and clip tables (recovered for hijacks). Transition table and state graph exist (data), meaning unknown |
| **Evidence** | Data in `vz.wad` (have parser for the table format); 360 code (consumers of the tables); PC and Xbox footage |
| **Missing** | Transition data, time-scale limits, partition masks, state graph |
| **Approximated** | Blend times and curve, rate clamp, upper-body mask |
| **Automatic validation** | Table decode tests; transition log coverage; foot-slide metric; mask diff per rig |
| **Needs visual check** | Transition timing against frame-stepped footage |
| **Complexity** | L |
| **Dependencies** | None in code; needs PC for data and Ghidra |
| **Order** | First (stage F1) |

### Clip selection and character coverage (A3)

| | |
|---|---|
| **Exists** | Tables `6802c321` / `e00b080c` / `182624e1` parsed and used for heroes, riders and hijacks; logs of each hero's picks |
| **Known** | Action key to handle to clip, most-specific row wins (inferred) |
| **Evidence** | Data; 360 code for locomotion action keys |
| **Missing** | Locomotion keys; a roster-wide coverage report |
| **Approximated** | Gait handles inferred from speed; footage-matched picks |
| **Automatic validation** | Coverage report (character × state: table clip, clip in use, agree?) |
| **Needs visual check** | Each disagreement before changing a pick |
| **Complexity** | M |
| **Dependencies** | F1's state graph work helps |
| **Order** | Report in F0; switches in F1 |

### Skeletons and transforms

| | |
|---|---|
| **Exists** | Node hierarchy from models; local = S·R·T (D3DX order); quaternions xyzw; skin matrices; clips bind by node name hash, with family rebinding |
| **Known** | Format recovered from samples and checked by skinning every fitting clip (PR #8, static-mask order fix) |
| **Evidence** | Data |
| **Missing** | 10 clips name nodes no model has; meaning of `trnm` signature |
| **Approximated** | None found in this audit |
| **Automatic validation** | Bind-pose skinning to identity (test exists); per-rig hierarchy dump; clip-to-rig binding coverage |
| **Needs visual check** | No, beyond the inspection tool |
| **Complexity** | S |
| **Dependencies** | None |
| **Order** | F0 (tooling only; no evidence of a fault) |

### Character movement and camera (M1 to M4, A5, A7)

| | |
|---|---|
| **Exists** | Rapier kinematic capsule, gaits, jump, swim, third-person camera |
| **Known** | Three `_HumanPhysics` fields read as speeds (inferred) |
| **Evidence** | 360 code (controller update), footage (pace, starts, stops, jump arcs, framing) |
| **Missing** | Acceleration, deceleration, air control, turn rate, step and slope rules, jump, root-motion use |
| **Approximated** | All of the above are invented |
| **Automatic validation** | Deterministic controller tests (time to speed, stop distance, turn time, jump apex) |
| **Needs visual check** | Footage pace and framing measurements |
| **Complexity** | L |
| **Dependencies** | P3 (frame order); interacts with A2 |
| **Order** | Stage F2 |

### Ragdolls (A6)

| | |
|---|---|
| **Exists** | Ragdolls built from bone lengths, joints with slack |
| **Known** | Nothing about the original's ragdolls |
| **Evidence** | Data search; 360 code |
| **Complexity** | M (research), unknown (implementation) |
| **Order** | After F2 |

### Vehicles (P1, P4)

| | |
|---|---|
| **Exists** | Rapier ray-cast vehicle; engine, gearbox, clutch, brakes, per-axle springs and friction from templates; boats, helicopters, planes, bikes with their own blocks; many examples (`drivetest`, `handlingtest`, `susptest`, `steertest`, `biketest`, `boattest`, `wheelspin`) |
| **Known** | Field names and values (recovered); their meaning (inferred); that the kit is Havok's (inferred from field names and Havok collision data) |
| **Evidence** | Data; 360 code; footage |
| **Missing** | The step order, equations and constants of the original |
| **Approximated** | Response shapes (easing, drag, climb help, track pull), every fallback constant |
| **Automatic validation** | Per-class numeric tests against an emulator of the recovered code |
| **Needs visual check** | Feel and footage checks after numeric match |
| **Complexity** | XL |
| **Dependencies** | P3; races and vehicle contracts must stay completable |
| **Order** | Research in F3, implementation in F5 |

### Explosions and forces (P2)

| | |
|---|---|
| **Exists** | Damage falloff from templates for objects and parts; particle forces recovered from the 360 code (PR #161) |
| **Known** | `Radius`, `Damage`, `MinDamage`, `MaxForce` per explosive (recovered) |
| **Missing** | How force is applied to rigid bodies and people |
| **Approximated** | Fixed push velocities; vehicle blast falloff |
| **Automatic validation** | Mass and distance sweep against an emulator |
| **Complexity** | M |
| **Dependencies** | Builds on PR #161's force work |
| **Order** | Stage F3 (high value, bounded scope) |

### Projectiles, collision materials, step rate (P3, P5, P6)

| | |
|---|---|
| **Exists** | Template-driven projectiles; collision classes; 60 Hz tick |
| **Missing** | Guidance law, drag, surface friction, the original step rate and order |
| **Complexity** | S each for research |
| **Order** | P3 research in F3 (it precedes any numeric comparison); P5 and P6 later |

### UI rendering and layout (U1, U3, U4, U6)

| | |
|---|---|
| **Exists** | Own SWF parser, AVM1 interpreter and player; Scaleform texture fonts; the game's shell, pause, loading, PDA and HUD scripts running; native widget runtime with resolution correction; gamma-correct UI blending |
| **Known** | Layout tables and correction rule (recovered from scripts); scale mode "show all" (inferred) |
| **Evidence** | Data (the movies); footage |
| **Missing** | Gradients, filters, blend modes beyond multiply, morph shapes, kerning, some HUD natives |
| **Automatic validation** | Feature census; fixed-resolution screenshot harness across aspect ratios |
| **Needs visual check** | Screenshot vs. footage frames |
| **Complexity** | M |
| **Dependencies** | None |
| **Order** | Census in F0; renderer work in F4 |

### UI behavior and input (U2, U5)

| | |
|---|---|
| **Exists** | Keyboard, mouse (rollover, press, hitTest) and pad in the Flash menus; the movies' own navigation; menu sounds from the shell bank |
| **Missing** | Prompts in the game's own widgets; PDA input |
| **Automatic validation** | Scripted navigation runs (the existing offscreen front-end key scripts) asserting the focused item after each key |
| **Complexity** | S to M |
| **Order** | F4 |

## Recommended order

Each stage is a set of rounds with one measurable outcome each.
Foundations first; content fixes only after the shared system is right.

**F0: Tools and census (no gameplay change).** Low risk, makes every later
step measurable.
1. Animation inspection tool (deliverable D3): pick a character and clip;
   play, pause, scrub, rate; skeleton with bone names and axes; reference
   vs. animated pose; per-bone local and world transforms; root motion
   trace; active clips and blend weights. Extend the existing
   `m2-alpha --model --clip --turn` path and the `clips` example rather
   than starting new.
2. Animation coverage report (D4): every hero and soldier rig × state;
   table clip vs. clip in use; missing and unbindable clips.
3. Flash feature census over every menu, PDA and HUD movie.
4. Fixed-resolution UI screenshot harness (D6) built on the offscreen
   front-end runs, at five resolutions.
5. Foot-slide metric and controller test scene (shared by F1 and F2).

**F1: Animation logic from the tables (A1, A2, A4, then A3).** Decode the
transition table and state graph; find their consumers in the 360 program;
apply per-transition blends, time-scale limits and partition masks with
today's values as fallback. Highest player-visible return for the effort,
and it fixes the shared cause behind most per-clip tuning so far.

**F2: Character controller (M1, M2, A5, A7).** Recover the movement update
from the 360 code; reproduce acceleration, deceleration, air control,
turning, jump; establish where root motion is used. Re-run the autoplay
contract set before merging.

**F3: Forces and the clock (P2, P3).** Trace the explosion's rigid-body
push (likely near PR #161's force code) and the physics step rate and
frame order. Bounded, and P3 is a prerequisite for F5's numeric tests.

**F4: UI renderer and prompts (U1, U2, U3).** Implement the Flash features
the census ranks highest; move prompts to the game's widgets; prove each
screen at all five resolutions.

**F5: Vehicle model (P1, then P4).** The largest item. Research round to
recover the vehicle step; a small emulator; then one vehicle class at a
time (cars, then tanks, then bikes, boats, helicopters, planes), each
gated on the race and vehicle contracts still completing.

**Later:** ragdolls (A6), decoder residuals (A8), collision materials
(P5), projectile guidance (P6), swimming (M4), text details (U6).

## Deliverables map

The owner's list, and where each lands.

| # | Deliverable | Stage |
|---|---|---|
| 1 | Physics subsystem inventory and fidelity roadmap | This file (first version) |
| 2 | Character animation architecture report | F1 output (`docs/fidelity/animation.md`) |
| 3 | Skeleton and animation validation tool | F0 |
| 4 | Animation coverage report across the roster | F0, refreshed after F1 |
| 5 | UI/menu architecture and behavior report | F4 output (`docs/fidelity/ui.md`) |
| 6 | Repeatable UI screenshot comparisons | F0 harness, used in F4 |
| 7 | Deterministic physics regression tests | F3 and F5 (extend the existing examples into tests) |
| 8 | Deterministic animation regression tests | F0 metrics, pinned in F1 |
| 9 | Documented original-versus-Rust comparisons | Each gap's verification field, closed table in GAPS.md |
| 10 | Remaining approximations and gaps | [GAPS.md](GAPS.md) |

## Checks that need the PC

This audit was made in the cloud. These need the PC (footage frames stay
there; Ghidra and the game's data are there):

| Check | Needed for |
|---|---|
| Decode `ab8fe34b` and the `ece70371` graph; survey the time-scale and mask columns | A1, A2, A4 |
| Ghidra: consumers of the animation tables; the blend function | A1, A2, A4, A3 |
| Ghidra: readers of `_HumanPhysics` (`b5353104`, `5cc8c4d9`, `6167d5fc`); the character movement update | M1, M2, A5, A7 |
| Ghidra: readers of `MaxForce`; explosion push on bodies | P2 |
| Ghidra: main loop and physics step call | P3 |
| Ghidra: the vehicle step, from the handling fields' readers | P1, P4 |
| Data search for ragdoll bodies and constraints | A6 |
| Flash feature census over the movies | U1, U6 |
| Footage: transition timing, run pace, start and stop, jump arcs, camera framing, menu frames | A1, M1, M2, M3, A5, U1, U2 |
