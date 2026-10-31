# Fidelity gaps

This is the tracker described in [README.md](README.md). It reflects
`main` at `de7ea9d` (2026-10-26), and code references point to that commit.

**Audit limits.** This first pass is based only on the code and the
project's notes. None of it has been checked against footage, the game's
data or the 360 program yet. So the **Original** fields mostly say
*Unknown*. Where they don't, they say which recovery step each claim has
reached (data recovered, consumer traced, verified; see the README).
Each record exists to show where the evidence is and how to get it. The
"PC check" column lists what must be done on the PC before a fix. That
covers footage frames, the game's data and the 360 program in Ghidra.

**Format.** Every record has the same eight fields: (1) Original, with its
evidence level, (2) Ours, (3) Difference, (4) Likely cause, (5) Evidence
available, (6) Correction, (7) Verification and (8) Risk.

## Summary, by priority

| Id | Gap | Class | Pri | PC check |
|---|---|---|---|---|
| [A1](#a1) | One fixed crossfade for every animation change | Animation/movement | P1 | Data, Ghidra, footage |
| [A2](#a2) | Clip playback rate vs. ground speed uses an invented clamp; tables' time-scale limits unread | Animation/movement | P1 | Data, Ghidra |
| [M1](#m1) | On-foot movement starts and stops instantly, with full air control | Animation/movement | P1 | Ghidra, footage |
| [M2](#m2) | Walk, run and sprint speeds fitted to clip bands, not taken from the hero templates | Behavioral | P1 | Data, Ghidra |
| [P1](#p1) | Vehicle handling: Rapier ray-cast vehicle with invented response shapes over the templates' fields | Behavioral | P1 | Ghidra, footage |
| [P2](#p2) | Explosion push on vehicles and people is a fixed invented velocity; `MaxForce` read but unused | Behavioral | P1 | Ghidra |
| [U1](#u1) | Flash renderer: gradients drawn in their first color, no filters, only multiply blend | Visual | P1 | Data census, footage |
| [U2](#u2) | Action prompts and loading lines drawn by our own egui text, not the game's widgets | Visual | P1 | Data, footage |
| [M3](#m3) | On-foot third-person camera distances and heights invented | Visual | P1 | Footage, data |
| [A3](#a3) | Locomotion clips chosen by three different paths; gait handles inferred from speed | Animation/movement | P2 | Data, Ghidra |
| [A4](#a4) | Upper/lower-body split derived from the skeleton; tables' `PartitionMask` unread | Animation/movement | P2 | Data, Ghidra |
| [A5](#a5) | Jump and landing: invented take-off speed, pelvis pinned during the clip | Animation/movement | P2 | Data, footage |
| [A6](#a6) | Ragdolls built from bone lengths with invented sizes | Animation/movement | P2 | Data, Ghidra |
| [A7](#a7) | Root motion always stripped; whether any state is animation-driven is unknown | Unknown | P2 | Ghidra, footage |
| [P3](#p3) | Simulation rate and step order not established from the original | Unknown | P2 | Ghidra |
| [P4](#p4) | Helicopter, boat and motorbike motion: invented easing over template fields | Behavioral | P2 | Ghidra, footage |
| [U3](#u3) | Aspect ratios: two scaling paths (Flash "show all" and widget correction) never compared across ratios | Visual | P2 | Footage |
| [U4](#u4) | Native HUD text animation and clock wipes drawn whole | Visual | P2 | Data, footage |
| [U5](#u5) | PDA buttons and mouse clicks not reaching the PDA in play (last audited 2026-10-21) | Functional bug | P2 | Play check |
| [A8](#a8) | Decoder residuals: wavelet seam steps on 14 clips, unknown header fields | Visual | P3 | Data, Ghidra |
| [P5](#p5) | Collision materials: friction and restitution per surface not read | Behavioral | P3 | Data, Ghidra |
| [P6](#p6) | Projectile guidance law, drag and range not established from code | Behavioral | P3 | Ghidra |
| [U6](#u6) | Flash text: one color per field, no kerning, HTML reduced to `<font color>` | Visual | P3 | Data census |
| [M4](#m4) | Swimming depths, float height, speed and stroke rate invented | Animation/movement | P3 | Footage, Ghidra |

Animation gaps A1 to A4 share one root cause. The game's animation tables
hold transition, time-scale and layering data that our runtime does not
read yet. Investigate them together (roadmap stage F1).

---

## Animation

### A1

**One fixed crossfade for every animation change** · Animation/movement · P1

1. **Original:** *Unknown.* We have located the data but not decoded it.
   The resident block holds an animation-transition table (`ab8fe34b`),
   and the human templates name a state graph (`HumanAnimationSystem`
   field `ece70371`) besides the action table (`6802c321`). These come
   from the hijack round H7 notes. Neither table is parsed. No consumer
   has been traced, so we don't know how the game picks a blend time or
   curve.
2. **Ours:** `m2_engine::Animator` fades linearly over 0.25 s for any
   looped change and over 0.1 s for one-shots, whatever the two clips are
   (`crates/m2-engine/src/anim.rs:651`, `:668`, `:677`, `:687`, `:782`).
   Invented.
3. **Difference:** Starts, stops, turns and stance changes all blend the
   same way. Where the original snaps or blends for longer, ours does not.
   This has not been measured against footage.
4. **Likely cause:** Shared. The transition data is not decoded, so every
   transition falls back to one constant.
5. **Evidence:** Data: `ab8fe34b` and the `ece70371` graph in `vz.wad`.
   360 code: the function that reads those tables and sets the blend.
   Footage: frame-stepped start, stop and stance changes (PC and Xbox
   frames). These measure the visible timing only and say nothing about
   the mechanism.
6. **Correction:** Decode `ab8fe34b` with the existing `animation_table`
   parser, which handles the same `UCFX` INFO/TYPE/VALU layout. That gives
   recovered data. Trace its consumer in the 360 program to establish what
   the columns mean (from/to keys, blend time, curve, sync). That traces
   the consumer. Then give `Animator::start` a blend time and curve looked
   up per transition, and keep today's values as the fallback when no row
   matches.
7. **Verification:** Unit tests that decoded rows match the values read on
   the PC. An emulator test of the traced selection function against our
   lookup, for the Verified level. A transition log in play
   (`from -> to, blend`) showing how many transitions use table values vs.
   the fallback, before (0%) and after. A frame-stepped footage comparison
   of three transitions (idle to run, run to stop, stand to aim) on two
   heroes, as a visible check only.
8. **Risk:** High, because every character uses the `Animator`. Guard it
   with the per-transition fallback and a roster-wide check of all three
   heroes and the soldier rigs before merging.

### A2

**Playback rate vs. ground speed** · Animation/movement · P1

1. **Original:** *Unknown* how the engine sets a clip's rate. Recovered
   data covers only the column names and types: both clip tables carry
   `MinTimeScale` and `MaxTimeScale` (f32) per row, in the vehicle table
   (`182624e1`) and the on-foot table (`e00b080c`)
   (`crates/m2-assets/src/animation_table.rs:573`, `:723`). We have not
   surveyed the values. The hijack rows sampled hold -1. That these
   columns limit a speed-matched rate is a guess from their names.
2. **Ours:** A moving gait plays at `moved / clip_speed`, clamped to the
   range 0.6 to 1.4 (`crates/m2-alpha/src/player.rs:1031`). Invented.
   Soldiers use similar rates (`crates/m2-alpha/src/npc.rs:1173`, `:1619`,
   `:2043`).
3. **Difference:** Feet slide when the movement speed falls outside the
   clamp, and clips may speed up or slow down where the original holds
   them. Not yet measured.
4. **Likely cause:** Shared. The tables' limits are unread, so the clamp
   stands in for them.
5. **Evidence:** Data: the two columns in every row. 360 code: where a
   clip's rate is computed from the character's speed, and what -1 means
   there.
6. **Correction:** Survey the column values across all rows (recovered
   data). Trace how the engine uses them (consumer traced). Then apply each
   row's limits to the clip that row selects.
7. **Verification:** First, a foot-slide metric: the world speed of each
   foot node while it touches the ground, averaged over 10 s of walking,
   running and sprinting, for each hero, along a fixed path as an
   automated test. Report it before and after. Second, unit tests of the
   limit lookup per row. Third, an emulator test of the traced rate
   function, for Verified.
8. **Risk:** Medium, because it touches every moving character. Guard it
   with the foot-slide metric across all rigs.

### A3

**Locomotion clip selection takes three different paths** ·
Animation/movement · P2

1. **Original:** We have recovered the table data: the layouts, the rows,
   and the chain from action key to handle (table `6802c321`) and from
   handle to clip (`e00b080c` on foot, `182624e1` for vehicles)
   (`crates/m2-assets/src/animation_table.rs:1`). Two rules are inferred
   from how the rows overlap: that this is the order the runtime looks
   them up, and that the most specific matching row wins. Hijacks driven
   this way match the hijack footage. That is behavioral evidence, but it
   is not a traced consumer. *Unknown:* which action keys the runtime
   issues for locomotion.
2. **Ours:** Gait handles are inferred from clip speeds alone
   (`crates/m2-alpha/src/scene.rs:144`, where a comment says "the handles'
   names are not known"). Clips are then chosen four ways: by table, by
   name, by measured root speed (`m2_engine::Locomotion::choose`,
   `crates/m2-engine/src/anim.rs:536`), or by matching footage. Jennifer's
   picks are kept even where the table disagrees (`scene.rs:172`).
3. **Difference:** Heroes and soldiers can play different clips from the
   ones the game would pick. So far, each correction has been made per
   character.
4. **Likely cause:** Shared. The locomotion action keys and the state
   graph that issues them are not decoded.
5. **Evidence:** Data: the table `6802c321` rows with stance `Upright`
   (columns `Stance`, `Action`, `LocomotionMask`), and the `ece70371`
   graph. 360 code: the code that sets the character's action key from
   its movement state. The existing log line "hero clips: <hero>: <slot>
   <hash> from row ..." already records where the table and the pick
   disagree.
6. **Correction:** Report first and change later. Build the coverage
   report (roadmap deliverable D4), which lists the table's clip and the
   clip in use for each character and state. Once the locomotion keys are
   traced in code, resolve every character through the tables. Keep a
   footage-matched pick only where evidence shows the game doesn't use the
   table's clip there.
7. **Verification:** Count the coverage report's disagreements before and
   after. Every disagreement that remains gets a written reason. An
   emulator test of the traced key selection, for Verified.
8. **Risk:** High for appearance, since this can replace picks Brendan has
   approved. Don't switch a pick without a footage check.

### A4

**Upper/lower-body layering** · Animation/movement · P2

1. **Original:** *Unknown.* For recovered data we only have the column
   names: table `6802c321` has `PartitionMask`, `ActionMask` and
   `LocomotionMask` columns (hijack round H7). We have not surveyed their
   values. That they say which body parts an action drives is a guess
   from the names.
2. **Ours:** The upper body is every node above the waist, found by
   walking from the hands (`crates/m2-engine/src/anim.rs:110`). There is a
   hard switch between masked and unmasked nodes. This rule is ours and
   invented.
3. **Difference:** Not measured. Possible symptoms: aiming or reloading
   while moving twists the spine differently from the original, and the
   mask has a hard edge at the waist.
4. **Likely cause:** Shared. The mask data is unread.
5. **Evidence:** Data: the mask columns. 360 code: how a partition mask
   is expanded to bones, and whether it carries weights.
6. **Correction:** Decode the masks (recovered data), then trace how they
   expand to bones (consumer traced). Use them in place of the derived
   split where they exist.
7. **Verification:** A unit test comparing each action's decoded mask
   with the derived one, reporting the node differences per rig. A
   footage check of aim-walk and reload-run, as a visible check only.
8. **Risk:** Medium, since it affects all armed characters. Guard it with
   the per-rig mask diff and the foot-slide metric.

### A5

**Jump and landing** · Animation/movement · P2

1. **Original:** *Unknown.* `_HumanPhysics` holds more fields than the
   three read as speeds (`crates/m2-engine/src/tuning.rs`). None has been
   identified as jump height or gravity.
2. **Ours:** Take-off at 4 m/s, about 0.8 m high
   (`crates/m2-alpha/src/player.rs:27`, invented). Gravity is 9.81
   (`crates/m2-engine/src/physics.rs:43`, assumed). The jump clip plays
   with the pelvis held at standing height (`Animator::play_leveled`).
   Landing hands back 0.1 s before its clip ends (`player.rs:32`,
   invented).
3. **Difference:** Jump height, airtime and landing timing may differ.
   Playtest 4 called the jump "a stiff 45-degree pose". Later rounds
   changed it, and it hasn't been rechecked against footage since.
4. **Likely cause:** Local to the jump, but with the same root as M1:
   the controller's constants are invented.
5. **Evidence:** Footage: jump arcs, which can be measured from frames
   against objects of known size. Data: the unidentified `_HumanPhysics`
   fields. 360 code: the character controller's jump.
6. **Correction:** Establish jump speed and gravity from the controller
   code, as part of M1's research. Then drop the pinned pelvis if the
   original lets the clip move it.
7. **Verification:** Measure apex height and airtime for a standing and a
   running jump, before and after. Compare with footage measurements
   (Approximated) or with an emulator of the traced controller (Verified).
8. **Risk:** Low to medium. Autoplay's hop helpers depend on jump height,
   so re-run the autoplay contracts that use them.

### A6

**Ragdolls** · Animation/movement · P2

1. **Original:** *Unknown.* The collision data (`PHY2`) has body records
   with mass and inertia (recovered data). We haven't established whether
   characters carry an authored ragdoll, with bodies and joint limits.
2. **Ours:** A ragdoll is built from the skeleton's bones, with sizes
   clamped to 0.07 to 0.7 m (`crates/m2-engine/src/physics.rs:1260`) and
   a joint slack of 2 cm (`:947`), both invented. The handover from
   animation at death and knock-down is our own design.
3. **Difference:** Playtest 4 reported "ragdolls stretch". By
   construction, limb limits and mass distribution differ from the
   original.
4. **Likely cause:** We don't read any ragdoll data.
5. **Evidence:** Data: search the character models' `PHY2` sections and
   other asset types for ragdoll bodies and constraints. 360 code: the
   ragdoll setup and the switch from animation to physics.
6. **Correction:** Start with a research round. If authored data exists,
   build ragdolls from it.
7. **Verification:** A ragdoll test scene with a 2 m drop and a blast at
   3 m. Report the maximum joint separation and the settle time per hero.
8. **Risk:** Medium, because deaths are frequent. Guard it with the test
   scene across every rig.

### A7

**Root motion vs. code-driven movement** · Unknown pending investigation ·
P2

1. **Original:** *Unknown.* From recovered data, locomotion clips carry
   their travel on the root track. We haven't established whether the
   game moves the character by the clip (root motion), by code, or each
   in different states such as hijacks, vaults, climb-outs and melee.
2. **Ours:** Root motion is always stripped horizontally, and code moves
   the character (`Pose::strip_root_motion`,
   `crates/m2-engine/src/anim.rs:236`). Hijacks place clips from the
   vehicle's origin with per-vehicle lifts fitted to footage
   (`crates/m2-alpha/src/hijack.rs` module docs, Approximated).
3. **Difference:** Unknown until the original is established. Possible
   symptoms: sliding during turns in place and stops, and the need for
   per-vehicle hijack offsets.
4. **Likely cause:** Shared, if the original uses root motion in some
   states.
5. **Evidence:** 360 code: where the character's position is advanced,
   and whether that code reads the root track.
6. **Correction:** Decide after the research. Make no change before it.
7. **Verification:** For each state traced to use root motion, compare
   root displacement with character displacement over the clip and report
   the error in cm.
8. **Risk:** High if changed broadly. Limit changes to the states the
   code shows, and re-run the hijack set.

### A8

**Decoder residuals** · Visual discrepancy · P3

1. **Original:** Recovered data (format). All 4,232 Havok 5.5 clips
   decode, and they agree with an independent decoder within 7.6e-6
   (PR #8). That shows two decoders read the data the same way. It is
   not verification against the original runtime's decompressor.
2. **Ours:** 14 clips show large steps at wavelet block seams. 10 clips
   name nodes that no model has. Clip header bytes 0x14 to 0x23 and the
   `trnm` layout signature are unknown (project notes, animation format).
3. **Difference:** A visible pop on those 14 clips, if they are played.
4. **Likely cause:** Either a decoding detail at block boundaries, or the
   data really steps there.
5. **Evidence:** Data; 360 code: the wavelet decompressor.
6. **Correction:** Trace the 360 decompressor's seam handling (consumer
   traced) and compare.
7. **Verification:** A seam-step metric per clip, defined as the largest
   per-frame rotation change at a seam compared with inside a block,
   measured before and after. An emulator test of the traced decompressor
   on those 14 clips, for Verified.
8. **Risk:** Low. The decoder has a test corpus, so re-run it.

## Character movement

### M1

**Instant starts and stops, full air control** · Animation/movement · P1

1. **Original:** *Unknown.* Footage would show whether the heroes ease
   into and out of a run. Nothing has been measured, and footage would
   show only the visible result, not the controller's internals.
2. **Ours:** The wanted horizontal velocity is the direction times the
   gait speed (`crates/m2-alpha/src/player.rs:824` to `:840`).
   `Physics::move_character` applies it as given every tick, even in the
   air (`crates/m2-engine/src/physics.rs:2999`). The controller is
   Rapier's kinematic capsule with a 0.5 m step and a 50-degree slope
   limit (`physics.rs:17`, `:39`, `:41`) and a 10 rad/s turn
   (`player.rs:37`). All three values are invented.
3. **Difference:** Every start, stop and direction change is immediate,
   and the character can steer freely in mid-air. The crossfades from A1
   hide part of this and add sliding.
4. **Likely cause:** Shared. The character controller is a conventional
   one, not the original's.
5. **Evidence:** 360 code: the player character's movement update, found
   through the readers of `_HumanPhysics` (`b5353104` / `5cc8c4d9` /
   `6167d5fc`). Footage: speed over time at the start and end of a run,
   by tracking a hero against the ground.
6. **Correction:** First a research round (consumer traced) to recover
   acceleration, deceleration, turn rate, air control, and the step and
   slope rules. Then reproduce them in the controller and keep Rapier only
   for the shape cast.
7. **Verification:** Deterministic controller tests on flat ground and a
   30-degree slope: time to top speed, stopping distance, time to turn
   180 degrees, and lateral change while airborne. Compare with an
   emulator of the traced code (Verified), or with footage-derived
   numbers where no code is traced (Approximated).
8. **Risk:** High. Autoplay, every mission's on-foot route, the climb
   driver (PmcCon004) and co-op all depend on this. Re-run the autoplay
   contract set after the change.

### M2

**Gait speeds** · Behavioral discrepancy · P1

1. **Original:** Recovered data: three `_HumanPhysics` field values per
   hero (`b5353104`, `5cc8c4d9`, `6167d5fc`). Inferred: that they are
   walk, run and sprint speeds (heroes notes). No consumer has been
   traced.
2. **Ours:** Jennifer walks at 1.4, runs at 5.0 and sprints at 7.2 m/s
   (`crates/m2-alpha/src/player.rs:20` to `:25`). These values were
   chosen inside the measured speed bands of the game's clips
   (Approximated). Other heroes are scaled by their template's values
   against hers.
3. **Difference:** The pace may differ from the original, which also adds
   to A2's sliding.
4. **Likely cause:** The field meanings are unconfirmed, and the absolute
   values aren't taken from the template.
5. **Evidence:** Data: the field values per hero. 360 code: what reads
   those fields. Footage: distance covered over time during a run and a
   sprint.
6. **Correction:** Trace the fields' readers (consumer traced) and use
   their values directly.
7. **Verification:** A unit test of the template read. An emulator test
   of the traced speed function, for Verified. A footage pace measurement
   (m/s over 5 s) before and after, as a visible check only.
8. **Risk:** Medium, because autoplay route timings depend on pace.
   Re-run the autoplay contract set.

### M3

**On-foot camera** · Visual discrepancy · P1

1. **Original:** *Unknown.* Vehicles read `CameraOffset` from their
   templates (recovered data; the field name was matched by hash). We
   don't know whether the on-foot camera has a template.
2. **Ours:** Eye height 1.6 m, distance 4.0 m, closest approach 0.6 m,
   plus shoulder offsets (`crates/m2-alpha/src/player.rs:65` to `:76`,
   `crates/m2-alpha/src/game.rs:268` to `:277`). None of these values
   has a recorded source, so treat them as invented.
3. **Difference:** This sets the framing of every on-foot frame. Not
   measured.
4. **Likely cause:** The values were chosen by eye.
5. **Evidence:** Footage: the hero's height and position on screen at
   rest and while aiming, at a known FOV. Data: a camera template for
   people, if there is one. 360 code: the on-foot camera update.
6. **Correction:** Use the data or code if either gives the rig.
   Otherwise fit the framing to footage and label it Approximated.
7. **Verification:** A screenshot at a fixed spot, pose and resolution,
   compared with a footage frame. Measure the screen positions of the
   hero's head and feet, with the error in percent of screen height.
8. **Risk:** Low, since only the camera changes. Check aiming and the
   camera's wall clearance afterwards.

### M4

**Swimming** · Animation/movement · P3

1. **Original:** *Unknown.* We haven't read or traced the game's swim
   depth, float height or swim speed. From recovered data: the stroke
   clip's own root speed is 0.81 m/s, its body lies flat with its highest
   node 0.11 m over the hips, and the `Swim` stance's `Idle` gives no
   clip of its own (`crates/m2-alpha/src/player.rs:51` to `:61`).
2. **Ours:** She starts swimming at 1.4 m of water and stands again
   below 1.15 m, with hysteresis between the two (`player.rs:38` to
   `:50`). She floats 0.35 m under the surface, swims at 1.6 m/s (the
   stroke played at twice its rate) and strokes at 0.35 times the rate
   while still (`player.rs:56` to `:62`). All of this is invented. The
   float height was set by eye.
3. **Difference:** Where she starts swimming, how high she floats, and
   how fast she swims relative to her stroke may differ from the
   original. Playing the stroke at twice its rate risks visible stroke
   speed-up and sliding. Not measured.
4. **Likely cause:** Local. No swim parameters are read from the data or
   traced in code.
5. **Evidence:** Footage: a swim from shore (depth at the change, height
   in the water, speed). Data: candidate fields in `_HumanPhysics` or a
   swim template (not searched). 360 code: the swim state's movement
   update.
6. **Correction:** Search the hero templates for swim fields, and trace
   the swim update in the 360 program. Without either, fit the depth,
   float and speed to footage and label them Approximated.
7. **Verification:** Measure the water depth where swimming starts and
   stops, the float height, and the swim speed (m/s over 5 s). Compare
   each before and after against footage, plus a foot/hand-slide check on
   the stroke.
8. **Risk:** Low. Guard the autoplay water exits (out of the sea, boat
   boarding) by re-running the contracts whose autoplay route crosses
   water, and VzaCon001's boat start.

## Physics

### P1

**Vehicle handling model** · Behavioral discrepancy · P1

1. **Original:** Recovered data: the vehicle templates' field values.
   Their names were found by hash match (`EngineTorque`, `GearOptRPM`,
   `TorqueFactorAtMinRPM`, `ClutchDelayTime`, `SteerMaxAngle`,
   `FrontWheelSuspStrength`, `FrontWheelFrictionSide`, `FrontBrake`,
   `HandbrakeInertia` and others, read in
   `crates/m2-engine/src/tuning.rs`, whose docs call them "guesses that
   hash to the fields' keys"). Inferred: what each field means. Also
   inferred: that the game runs Havok's vehicle kit, since the names fit
   it and the collision data is Havok 5.5. No consumer has been traced.
2. **Ours:** Rapier's `DynamicRayCastVehicleController`
   (`crates/m2-engine/src/physics.rs:1840`), driven by those fields under
   their inferred meanings. On top of that sit invented response shapes
   and fallbacks: top-speed easing, ground drag, climb help on hills,
   track pull and a parked hold (`physics.rs:879` to `:1035`).
3. **Difference:** Playtests have raised speed ("vehicles feel slower"),
   hill climbing and bikes on their side. Each was tuned separately.
4. **Likely cause:** Shared. The handling model is a conventional one fed
   the original's parameters, so its response can't match.
5. **Evidence:** Data: all template fields (we have these). 360 code: the
   vehicle step, found from the readers of these fields' hashes. Footage:
   the tank, jeep and boat clips already in use
   (`gameplay/tank-vs-video.md`), for visible checks.
6. **Correction:** Research comes first (stage F5). Recover the per-step
   order (engine, transmission, brakes, steering, suspension, friction,
   aerodynamics), the equations and the constants (consumer traced). Write
   a small emulator of the recovered code, as was done for particle
   forces. Reimplement it independently, and keep Rapier only for the
   chassis body and collision.
7. **Verification:** Deterministic tests per vehicle class: time from 0
   to top speed, top speed, braking distance from 20 m/s, turn radius at
   10 m/s, and suspension settle. Compare numerically with the emulator,
   reporting the maximum absolute and relative error, accumulated over
   600 steps. The existing examples (`drivetest`, `handlingtest`,
   `susptest`, `steertest`, `biketest`) become the regression harness.
8. **Risk:** Very high. Races, chases, autoplay contract completion and
   co-op all depend on handling. Change one vehicle class at a time, and
   re-run the race contracts and vehicle contracts before each merge.

### P2

**Explosion push** · Behavioral discrepancy · P1

1. **Original:** Recovered data: explosion templates give `Radius`,
   `Damage`, `MinDamage` and `MaxForce`, with names matched by hash
   (`crates/m2-engine/src/weapons.rs:977`). *Unknown:* how the runtime
   turns them into a push on rigid bodies, since no consumer is traced.
   The particle force kinds were traced in the 360 code, and the Rust
   particle step matches an emulator of it (PR #161, open). That covers
   particles only.
2. **Ours:** A blast that wrecks a vehicle throws it with a fixed
   velocity change of 4 m/s outward and 6 m/s up, times its mass
   (`crates/m2-alpha/src/combat.rs:4486`). People get a fixed push
   (`BLAST_PUSH`, `combat.rs:506`). Blast damage to vehicles uses its own
   falloff (`BLAST_ON_VEHICLE`, `combat.rs:538`). All three are invented.
   `MaxForce` is read but never used.
3. **Difference:** Every explosion throws things the same way, whatever
   the explosive, distance or mass. Vehicles that aren't wrecked aren't
   pushed at all.
4. **Likely cause:** Shared. The force model was never established.
5. **Evidence:** 360 code: the explosion's effect on rigid bodies. Follow
   the readers of `MaxForce`, and check whether they share the force-kind
   code traced for particles. Data: the values per explosive.
6. **Correction:** Trace the falloff and mass dependence (consumer
   traced). Apply the result to vehicles, props, ragdolls and debris
   through one function.
7. **Verification:** A deterministic test with bodies of 2,000 kg and
   40,000 kg at 2, 5 and 10 m from a grenade, an RPG and the satellite
   bomb, comparing final velocity with an emulator of the traced function
   (Verified). A footage check of the gate and car explosions already
   catalogued, as a visible check only.
8. **Risk:** Medium to high, because it affects destruction, where wrecks
   end up, and missions that need a vehicle intact. Re-run the
   destruction contracts.

### P3

**Simulation rate and step order** · Unknown pending investigation · P2

1. **Original:** *Unknown* for physics. Particles only: their step runs on
   a 60 Hz clock that was traced in the 360 code, and the Rust step
   matches an emulator of it 2,000/2,000 (Verified, PR #161, open). The
   physics step rate, its substeps, and the order of game logic, physics
   and animation within a frame are not established.
2. **Ours:** One fixed 60 Hz tick (`crates/m2-alpha/src/main.rs:129`),
   with Rapier stepped once per tick.
3. **Difference:** Nothing visible so far. The rate matters for the
   numerical comparisons in P1, P2 and M1.
4. **Likely cause:** Not established.
5. **Evidence:** 360 code: the main loop and the physics world's step
   call.
6. **Correction:** Record the original's rate and order (consumer traced),
   and change ours only if they differ.
7. **Verification:** A documented frame-order table, and emulator tests
   run at the original rate.
8. **Risk:** Low to establish, but high to change, since every system
   steps on this tick. Re-run the full autoplay set before changing it.

### P4

**Helicopters, boats and motorbikes** · Behavioral discrepancy · P2

1. **Original:** Recovered data: each kind's template block, with field
   names matched by hash. Helicopters use `_HelicopterPhysics`
   (`maxYawSpeed`, `PitchAccel`, `RollAccel`, ...), boats use
   `_BoatPhysics` (engine and the water drags `WaterDragFwd`, `Side` and
   `Up`), and motorbikes use the `_CarPhysicsV2` `Bike*` fields
   (`crates/m2-engine/src/tuning.rs:58` to `:64`, `:72`, `:101`, `:175`).
   Inferred: what each field means. *Unknown:* how the runtime applies
   them, since no consumer is traced. `BikeWheeliePitch` and
   `BikeWheelieSideDamp` are named but not read (`tuning.rs:196`).
2. **Ours:** Each kind reads its block, then shapes the response with
   invented easing (`crates/m2-engine/src/physics.rs:979` to `:1035`).
   For helicopters: `YAW_SHARE` (inferred, "check against footage"),
   `FLY_EASE`, `CLIMB_EASE` and `LEAN_EASE`. For boats: `FLOAT_DEPTH`,
   `BOAT_ROCK`, `BOAT_TURN`, `BOAT_TOP`, planing (`PLANE_SPEED`,
   `PLANE_RISE`, `PLANE_TRIM`, fitted by eye to the videos) and fallback
   `BOAT` tuning. For bikes: fallback `BIKE` tuning and a 45-degree
   maximum lean, `BIKE_LEAN`. Each is labeled Invented or Inferred in the
   code.
3. **Difference:** Not measured against footage. Playtests reported a
   bike lying on its side. Helicopter turn and climb rates, boat planing
   and bike lean may all differ from the original.
4. **Likely cause:** Shared with P1. These are conventional responses fed
   the original's parameters.
5. **Evidence:** Data: the template blocks (we have these). 360 code: each
   kind's step, from the readers of its fields. Footage: the Avenger and
   Osprey reference (`falBXvRXBAU`), the boat rides in the first mission,
   and the bike playtest recordings, for visible checks.
6. **Correction:** Do this after P1 establishes the shared vehicle step,
   one kind at a time. Trace the kind's step (consumer traced), write an
   emulator, and reimplement it independently.
7. **Verification:** Deterministic tests per kind against its emulator:
   for helicopters, yaw rate at full stick, climb rate and lean angle at
   full stick; for boats, top speed, stopping distance and planing height
   at 15 m/s; for bikes, lean at full lock and recovery after a landing.
   The existing `biketest`, `boattest` and `boatsteer` examples become
   the harness.
8. **Risk:** High for the contracts that depend on them: PirCon001's
   jetski race, the boat stages of AllCon002 and VzaCon001's start, and
   the helicopter contracts (PmcCon004's climb driver). Re-run those
   contracts before each merge.

### P5

**Collision materials** · Behavioral discrepancy · P3

1. **Original:** *Unknown.* From recovered data: the `PHY2` triangles
   carry no surface word (it is 0 on all 1,040,652), and the shapes carry
   a collision class (`crates/m2-assets/src/physics.rs` module docs).
   Render materials name a surface word (`METAL`, `TERRAIN_DIRT`, ...),
   which is also recovered data (`crates/m2-engine/src/surface.rs:1`). We
   haven't found where the runtime's friction and restitution per
   surface or per class come from.
2. **Ours:** A model's shapes take the surface of most of its solid drawn
   area (`surface.rs:7` to `:11`, inferred). That surface only picks
   impact effects and sounds. Friction is set per use: 0.6 for chassis
   and turret (`crates/m2-engine/src/physics.rs:1826`, `:1835`), 0.9 for
   wrecks (`WRECK_FRICTION`, `:952`) and dropped bodies (`:2711`), and
   `DEBRIS_FRICTION` 0.8 (`:1314`). Restitution is left at Rapier's
   default. All invented.
3. **Difference:** Wrecks, crates and debris slide and bounce the same on
   road, dirt, grass and metal. Not measured against footage.
4. **Likely cause:** No physical material data is read, and its location
   is unknown.
5. **Evidence:** Data: the `PHY2` collision class and the render
   materials' surface words, plus any material table in the template
   database (not yet searched). 360 code: the code that sets body or
   contact friction.
6. **Correction:** Find the material table or the code that assigns
   friction and restitution (consumer traced), and apply it by surface
   or class through one function.
7. **Verification:** A deterministic slide test: a wreck and a crate
   given 10 m/s on road, dirt and grass. Measure slide distance and bounce
   height against an emulator of the traced code (Verified), or against
   footage where available (Approximated).
8. **Risk:** Low to medium. Wrecks blocking roads, and crates pushed in
   contracts, could change. Re-run the destruction and delivery contracts.

### P6

**Projectiles** · Behavioral discrepancy · P3

1. **Original:** Recovered data, with names matched by hash: speed and
   `HumanBoost`, `GravityScale`, `Damage`, `DamageMinimum`, the range
   falloff, and for homing weapons `LockOnTime`, the lock range, the
   projectile's turn rate and its widest angle off the nose
   (`crates/m2-engine/src/weapons.rs:1` to `:30`, `:55` to `:66`,
   `:1028`, `:1121`). Inferred: what each field means (the lock range's
   400 is marked inferred). *Unknown:* the guidance law, air drag, how
   laser designation guides weapons, and the runtime's projectile
   lifetime or range, since no consumer is traced.
2. **Ours:** Rounds fly straight, or fall at 9.81 times `GravityScale`
   (`crates/m2-alpha/src/combat.rs:4240`, `:4332`). A homing rocket turns
   its velocity toward the target's origin, no faster than its turn rate,
   by interpolating its direction (`combat.rs:4292` to `:4305`). It
   ignores the max-angle field and has no drag. Rockets expire at 400 m
   (`ROCKET_RANGE`, `combat.rs:458`). The satellite bomb's falloff is
   invented (`combat.rs:510` to `:518`). The pursuit law, the missing
   drag and the range are our own design.
3. **Difference:** Not measured. Homing rockets may curve differently
   (pure pursuit of the target's origin, versus whatever the original
   steers toward), and long shots may fall or slow differently.
4. **Likely cause:** No projectile controller has been traced.
5. **Evidence:** 360 code: the projectile controller, found from the
   readers of the turn-rate, max-angle and gravity-scale hashes. Footage:
   the weapons reference (`pRsuaYT7cup`) for visible paths only.
6. **Correction:** Trace the controller (consumer traced), then implement
   its guidance, drag and expiry in one projectile step.
7. **Verification:** Deterministic flight tests against an emulator of the
   traced controller: a homing rocket on a target crossing at 20 m/s, a
   grenade-launcher round at 45 degrees, and a rifle round at 300 m.
   Report path error and time of flight.
8. **Risk:** Medium. Autoplay's anti-vehicle and helicopter fights depend
   on hit rates, so re-run the contracts that use launchers.

## UI

### U1

**Flash renderer coverage** · Visual discrepancy · P1

1. **Original:** Recovered data: the menu, pause, loading, PDA and HUD
   pop-up movies themselves, which we parse and run. We haven't yet
   counted which SWF features they use. How a feature should look comes
   from the public SWF specification and the published Scaleform tag
   list. That is a documented format, not a traced consumer, so treat the
   Scaleform player's exact rendering of a feature as Inferred until it
   is checked against footage.
2. **Ours:** Gradients are drawn in their first color. Morph shapes are
   not drawn. `PlaceObject3` filters (glow, drop shadow, blur) are
   ignored. Of the blend modes, only multiply is drawn
   (`crates/m2-assets/src/flash/mod.rs:26`, `:265`). That module comment
   predates text and ActionScript support and should be refreshed.
3. **Difference:** Wherever the movies use those features, colors,
   highlights and selection glows differ. The size of the effect is
   unknown until the census is done.
4. **Likely cause:** Shared, because one renderer serves every movie.
5. **Evidence:** Data: a census of the movies (the count of gradient
   fills, filters, blend modes and morph shapes per movie). Footage: menu
   and pause frames, for visible checks.
6. **Correction:** Run the census first. Then implement features in order
   of how many on-screen elements each affects. Gradients are expected to
   come first.
7. **Verification:** Census numbers per movie (features drawn vs. used),
   before and after. Fixed-resolution screenshots of the main menu, pause
   menu and PDA compared with footage frames, measuring color difference
   per region (a visible check).
8. **Risk:** Low to medium. A renderer-level change affects every movie,
   so check every movie's screenshot, not only the one being fixed.

### U2

**Prompts and loading lines in our own text** · Visual discrepancy · P1

1. **Original:** Recovered data: the game's prompt strings (for example
   `Hijack %s`), its button-glyph page, and the loading screen's hints and
   their stored order. Footage shows the game drawing its prompts in its
   own font, as visual evidence. *Unknown:* which HUD widget and engine
   event the runtime uses to show the general action prompts.
2. **Ours:** The "what the player can do next" prompts and the loading
   lines are drawn in egui's bundled fonts
   (`crates/m2-alpha/src/ui.rs:5` to `:8`). The hijack's own button
   prompts already use the game's HUD.
3. **Difference:** A different font, size and position every time a
   prompt shows.
4. **Likely cause:** The game's prompt widget and its event are not wired
   up for most actions.
5. **Evidence:** Data: the HUD layout's prompt widgets and the text
   table's prompt strings. Footage: a prompt's position and size, for
   visible checks. 360 code: the engine side of the prompt event, if the
   layouts don't settle it.
6. **Correction:** Route prompts through the game's own widget, using the
   text table's strings, and keep egui only for developer overlays.
7. **Verification:** A list of the prompts drawn by egui vs. by the game's
   widget, before and after. A screenshot of each prompt compared with
   footage.
8. **Risk:** Low. Autoplay reads prompt state, not pixels (to be
   confirmed in `autoplay.rs` before the change).

### U3

**Aspect ratios and resolutions** · Visual discrepancy · P2

1. **Original:** Recovered from the game's scripts (data, read through
   how `MrxGuiBase` calls the widget natives): the native widget layout
   rule. Child boxes sit on a 640×480 virtual screen, and top-level
   widgets keep their anchor's screen fraction, with sizes scaled by
   H/480 (`crates/m2-game/src/gui.rs`, `Correction`). Inferred: that
   full-screen Flash movies use Scaleform's default "show all" mode. The
   game's video options offer a 16:9 or 4:3 Aspect Ratio. *Unknown:* how
   the runtime fits movies at ratios other than 16:9 and 4:3 (16:10,
   21:9).
2. **Ours:** Full-screen Flash movies are fitted in "show all" mode,
   centered and keeping their stage's shape. The pause menu's movie is
   the exception: it is stretched over its widget's box
   (`crates/m2-alpha/src/gamehud.rs:4739` to `:4743`). Native widgets use
   the anchor correction. The 4:3 option applies (9b836cb). The footage
   matches were each made at one resolution (HUD round 2: health ±1 px,
   weapon box ±3 px).
3. **Difference:** Not measured beyond those single-resolution matches.
   At other ratios, Flash and native elements could drift apart (the two
   paths scale differently), and the pause menu could stretch.
4. **Likely cause:** Two scaling paths that nobody has compared with each
   other across ratios.
5. **Evidence:** Footage at 16:9, plus any 4:3 footage, for visible
   checks. 360 code: the movie-widget fit, if footage can't settle it.
6. **Correction:** None yet. First measure with the screenshot harness
   (F0). Fix only if an element breaks its own rule at some ratio, and
   fix it in the shared scaling path, not per screen.
7. **Verification:** The same screens (main menu, pause menu, HUD on
   foot and in a vehicle, PDA) at 1280×720, 1920×1080, 2560×1440,
   1920×1200 and 1024×768. Report each element's position in virtual
   units. Every element must keep its anchor's fraction, and the result
   becomes an automated screenshot test.
8. **Risk:** Low. Guard it with all five resolutions on every change, so
   one ratio isn't fixed by breaking another.

### U4

**Native HUD text animation and clock wipes** · Visual discrepancy · P2

1. **Original:** Recovered from the game's scripts (data): the HUD
   scripts call `AnimateText`, `AnimateSprite` and the clock-wipe and pie
   natives on widgets. *Unknown:* what the runtime draws for each call
   (text reveal order and timing, wipe direction), since no consumer is
   traced.
2. **Ours:** `AnimateSprite` runs frame ranges over time
   (`crates/m2-game/src/gui_runtime.rs:2062`). `AnimateText` is recorded
   as a call only, and the text is drawn whole
   (`gui_runtime.rs:2054` to `:2060`). Clock wipes draw as their full
   image, as the same comment says. `SetImagePieSliceRender` stores its
   arguments (`gui_runtime.rs:1993`). Whether the drawing applies them in
   every case was not checked in this audit. Other unimplemented natives
   are listed at run time in `Screen::unknown`. The `gamehud.rs:16`
   module comment still lists the PDA as not drawn, which is stale.
3. **Difference:** Text that the original reveals or animates appears at
   once, and timers or cooldowns drawn as clock wipes show full. Which
   HUD elements this affects in play hasn't been counted.
4. **Likely cause:** Local to the widget runtime. These natives were
   stubbed.
5. **Evidence:** Data: the HUD scripts' calls and their arguments.
   Footage: a text reveal and a wipe (timing and direction). 360 code:
   the natives' implementations, if footage can't settle them.
6. **Correction:** Run a full mission and collect `Screen::unknown`, plus
   the stubbed natives that get called. Mark each one used or unused by
   the HUD scripts. Implement the used ones in the widget runtime, in
   order of how often they're called.
7. **Verification:** The count of stubbed natives called per mission,
   before and after. Frame-stepped screenshots of one text animation and
   one wipe compared with footage.
8. **Risk:** Low. Changes stay inside `gui_runtime`. Re-run the HUD
   scripts' error count (0 today on the first mission) as the guard.

### U5

**PDA input** · Functional bug · P2

1. **Original:** Recovered from the game's scripts (data): the PDA's
   input handler, `MrxGuiPda._HandleInput` (script `97f9c4ee`). In map
   mode it passes `LeftAnalogX`/`Y` to the map movie. The movie handles
   the PDA's buttons. Footage shows the PDA navigated by pad.
2. **Ours:** The map follows the pad's stick (`ControllerInput`,
   `crates/m2-alpha/src/gamehud.rs:1632`; 66bc531). An offscreen harness
   types keys and clicks into the PDA's movie for screenshots
   (`--pda-key`, `--pda-click`, `crates/m2-alpha/src/main.rs:288`). The
   2026-10-21 audit found that, in live play, button presses and mouse
   clicks never reach the PDA. That hasn't been re-measured on
   `de7ea9d`.
3. **Difference:** In play, the player can open the PDA and pan the map
   but can't use its buttons (as of the last audit).
4. **Likely cause:** Live key and mouse events aren't forwarded to the
   PDA's movie the way the harness forwards them. Not confirmed.
5. **Evidence:** Data: the PDA scripts and movie (`topbar`). A play check
   on the PC.
6. **Correction:** Re-check first. If it's still broken, forward live
   keys, pad buttons and mouse events to the PDA's movie by the same path
   the harness uses. This item is already in the project's to-do queue
   (PDA button and mouse input), so take it from there instead of
   duplicating it.
7. **Verification:** A scripted run that opens the PDA, selects a
   contract by key, by pad and by click, and asserts the movie's selected
   contract after each input. Run it before and after.
8. **Risk:** Low to medium. Input routing is shared with the pause menu
   and the support menu, so re-run their scripted navigation runs too.

### U6

**Flash text details** · Visual discrepancy · P3

1. **Original:** Recovered data: the text fields (`DefineEditText`) and
   the Scaleform texture fonts (`DefineFont3` plus `FontTextureInfo`),
   including any kerning tables and HTML in the fields. How fields should
   lay out comes from the public SWF specification, which is a documented
   format, not a traced consumer. *Unknown:* which fields use kerning,
   multiple colors or other HTML tags.
2. **Ours:** Layout handles alignment, margins, leading and word wrap,
   and leaves no leading under the last line (inferred from footage).
   HTML tags are stripped. The field takes the color of its first
   `<font color>` only, so one color per field. Kerning tables aren't
   applied (`crates/m2-assets/src/flash/text.rs:17`, `:446` to `:448`,
   `:562` to `:569`).
3. **Difference:** Fields with mixed colors, bold or sized runs, or kerned
   pairs render differently. How often this happens is unknown until the
   census.
4. **Likely cause:** Local to the text layout, which implements a subset
   of the SWF text model.
5. **Evidence:** Data: a census of text fields (HTML tags used, mixed
   colors, and kerning tables in the font movies). Footage: menu and HUD
   text, for visible checks.
6. **Correction:** Run the census, then implement only the features the
   movies use (likely per-run color first, then kerning), in the shared
   layout.
7. **Verification:** Census numbers before and after. A screenshot of
   each affected field compared with footage, and unit tests of the
   layout on hand-built fields.
8. **Risk:** Low to medium. Text metrics feed wrapping and line counts,
   so check every movie's screenshot at the five resolutions for changed
   line breaks.

## Closed

None yet.
