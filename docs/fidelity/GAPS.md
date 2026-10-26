# Fidelity gaps

The tracker described in [README.md](README.md). State of `main` at
`de7ea9d` (2026-10-26). Code references are to that commit.

**Audit limits.** This first pass reads the code and the project's notes
only. "Original" fields are therefore mostly *Unknown* or *Inferred*: the
point of each record is to say where the evidence is and how to get it.
The column "PC check" lists what must be done on the PC (footage frames,
the game's data, the 360 program in Ghidra) before a fix.

## Summary, by priority

| Id | Gap | Class | Pri | PC check |
|---|---|---|---|---|
| [A1](#a1) | One fixed crossfade for every animation change | Animation/movement | P1 | Data, Ghidra, footage |
| [A2](#a2) | Clip playback rate vs. ground speed uses an invented clamp; tables' time-scale limits unread | Animation/movement | P1 | Data, Ghidra |
| [M1](#m1) | On-foot movement starts and stops instantly, with full air control | Animation/movement | P1 | Ghidra, footage |
| [M2](#m2) | Walk, run and sprint speeds fitted to clip bands, not read from the hero templates' meaning | Behavioral | P1 | Data, Ghidra |
| [P1](#p1) | Vehicle handling: Rapier ray-cast vehicle with invented functional forms over the templates' fields | Behavioral | P1 | Ghidra, footage |
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
| [P4](#p4) | Helicopter, boat, plane and bike motion: invented easing over template fields | Behavioral | P2 | Ghidra, footage |
| [U3](#u3) | Aspect ratios: two scaling paths (Flash "show all" and widget correction) never compared across ratios | Visual | P2 | Footage |
| [U4](#u4) | Native HUD animation natives partly missing (sprite/text animation, clock wipes) | Visual | P2 | Data |
| [U5](#u5) | PDA does not receive button presses or mouse clicks | Functional bug | P2 | (tracked in the to-do queue) |
| [A8](#a8) | Decoder residuals: wavelet seam steps on 14 clips, unknown header fields | Visual | P3 | Data, Ghidra |
| [P5](#p5) | Collision materials: friction and restitution per surface not read | Behavioral | P3 | Data, Ghidra |
| [P6](#p6) | Projectile guidance and drag laws not established from code | Behavioral | P3 | Ghidra |
| [U6](#u6) | Flash text: no kerning; HTML limited to `<font color>` | Visual | P3 | Data census |
| [M4](#m4) | Swimming depths, float height and stroke rate invented | Animation/movement | P3 | Footage |

Animation gaps A1 to A4 share one root: the game's animation tables carry
transition, time-scale and layering data that our runtime does not read
yet. They should be investigated together (roadmap stage F1).

---

## Animation

### A1

**One fixed crossfade for every animation change** · Animation/movement · P1

1. **Original:** *Unknown; data exists.* The resident block holds an
   animation-transition table (`ab8fe34b`) and the human templates name a
   state graph (`HumanAnimationSystem` field `ece70371`) besides the
   action table (`6802c321`) (notes from hijack round H7). Neither is
   parsed. How the game picks a blend time and curve is not established.
2. **Ours:** `m2_engine::Animator` fades linearly over 0.25 s for any
   looped change and 0.1 s for one-shots, whatever the two clips are
   (`crates/m2-engine/src/anim.rs:651`, `:668`, `:677`, `:687`, `:782`).
3. **Difference:** Starts, stops, turns and stance changes all blend the
   same way. Where the original snaps or blends longer, ours does not
   (not yet measured against footage).
4. **Likely cause:** Shared. The transition data is not decoded, so every
   transition falls back to one constant.
5. **Evidence:** Data: `ab8fe34b` and the `ece70371` graph in `vz.wad`.
   360 code: the function that reads those tables and sets the blend.
   Footage: frame-stepped start, stop and stance changes (PC and Xbox
   frames) for timing only.
6. **Correction:** Decode `ab8fe34b` with the existing `animation_table`
   parser (same `UCFX` INFO/TYPE/VALU layout). Find its consumer in the
   360 program to establish what the columns mean (from/to keys, blend
   time, curve, sync). Give `Animator::start` a blend time and curve
   looked up per transition, keeping today's values as the fallback when
   no row matches.
7. **Verification:** Unit tests that each decoded row gives the expected
   duration and curve. A transition log in play (`from -> to, blend`)
   showing how many transitions use table values vs. the fallback, before
   (0%) and after. Frame-stepped footage comparison of three transitions
   (idle to run, run to stop, stand to aim) on two heroes.
8. **Risk:** High. Every character uses the `Animator`. Guard with the
   per-transition fallback and a roster-wide check (all three heroes and
   the soldier rigs) before merging.

### A2

**Playback rate vs. ground speed** · Animation/movement · P1

1. **Original:** *Inferred from data columns.* Both clip tables carry
   `MinTimeScale` and `MaxTimeScale` per row: the vehicle table
   (`182624e1`) and the on-foot table (`e00b080c`)
   (`crates/m2-assets/src/animation_table.rs:573`, `:723`). How the engine
   applies them is not established.
2. **Ours:** A moving gait plays at `moved / clip_speed`, clamped to
   0.6 to 1.4 (invented) (`crates/m2-alpha/src/player.rs:1031`).
   Soldiers use a similar rate (`crates/m2-alpha/src/npc.rs:1173`,
   `:1619`, `:2043`).
3. **Difference:** Feet slide when the movement speed leaves the clamp,
   and clips may speed up or slow down where the original holds them.
4. **Likely cause:** Shared. The tables' limits are unread; the clamp is a
   stand-in.
5. **Evidence:** Data: the two columns per row (values not yet surveyed;
   hijack rows hold -1). 360 code: where a clip's rate is computed from the
   character's speed.
6. **Correction:** Survey the columns' values across all rows. Find how
   the engine uses them (and what -1 means). Apply the row's limits to the
   clip the row selected.
7. **Verification:** A foot-slide metric: for each foot node, its world
   speed while in ground contact, averaged over a 10 s walk, run and
   sprint for each hero (an automated test on a fixed path). Report the
   before and after numbers. Unit test the limit lookup per row.
8. **Risk:** Medium. Touches every moving character. Guard with the
   foot-slide metric across all rigs.

### A3

**Locomotion clip selection takes three different paths** ·
Animation/movement · P2

1. **Original:** *Recovered in part.* Characters are animated by key: a
   stance and action resolve through table `6802c321` to a handle, and the
   handle through `e00b080c` (on foot) or `182624e1` (vehicles) to a clip
   (`crates/m2-assets/src/animation_table.rs:1`). Which action keys
   locomotion uses is not known.
2. **Ours:** Gait handles are inferred from clip speeds alone
   (`crates/m2-alpha/src/scene.rs:144`, "the handles' names are not
   known"). Clips are then chosen by table, by name, by measured root speed
   (`m2_engine::Locomotion::choose`, `crates/m2-engine/src/anim.rs:536`)
   or matched to footage (Jennifer's picks are kept even where the table
   disagrees, `scene.rs:172`).
3. **Difference:** Heroes and soldiers can play different clips from the
   ones the game would pick; each correction so far has been per character.
4. **Likely cause:** Shared. The locomotion action keys and the state
   graph that issues them are not decoded.
5. **Evidence:** Data: table `6802c321` rows with stance `Upright`
   (columns `Stance`, `Action`, `LocomotionMask`), the `ece70371` graph.
   360 code: the code that sets the character's action key from its
   movement state. The existing log "hero clips: <hero>: <slot> <hash> from
   row ..." already records where table and pick disagree.
6. **Correction:** Report first, change later. Build the coverage report
   (roadmap deliverable D4) listing, per character and state, the table's
   clip and the clip in use. Once the locomotion keys are established,
   resolve every character through the tables and keep footage-matched
   picks only where evidence shows the table is not what the game uses.
7. **Verification:** The coverage report's count of disagreements, before
   and after; each remaining disagreement has a written reason.
8. **Risk:** High for appearance: it can replace picks Brendan has
   approved. Do not switch a pick without a footage check.

### A4

**Upper/lower-body layering** · Animation/movement · P2

1. **Original:** *Unknown; data exists.* Table `6802c321` has
   `PartitionMask`, `ActionMask` and `LocomotionMask` columns (hijack notes,
   round H7). They likely say which body parts an action drives.
2. **Ours:** The upper body is every node above the waist, found by
   walking from the hands (`crates/m2-engine/src/anim.rs:110`), with a
   hard switch between masked and unmasked nodes.
3. **Difference:** Not measured. Candidate symptoms: aiming or reloading
   while moving twists the spine differently from the original, and a hard
   mask edge at the waist.
4. **Likely cause:** Shared. The mask data is unread.
5. **Evidence:** Data: the mask columns. 360 code: how a partition mask is
   expanded to bones (and whether it carries weights).
6. **Correction:** Decode the masks and use them in place of the derived
   split where present.
7. **Verification:** Unit test: decoded mask per action vs. the derived
   one, reporting the node differences per rig. Footage check of aim-walk
   and reload-run.
8. **Risk:** Medium; affects all armed characters.

### A5

**Jump and landing** · Animation/movement · P2

1. **Original:** *Unknown.* `_HumanPhysics` holds fields beyond the three
   read as speeds (`crates/m2-engine/src/tuning.rs`); none is identified as
   jump height or gravity.
2. **Ours:** Take-off at 4 m/s (about 0.8 m, invented,
   `crates/m2-alpha/src/player.rs:27`), gravity 9.81
   (`crates/m2-engine/src/physics.rs:43`), the jump clip played with the
   pelvis held at standing height (`Animator::play_leveled`), landing
   handed back 0.1 s before its clip ends (`player.rs:32`).
3. **Difference:** Jump height, airtime and landing timing may differ
   (playtest 4 called the jump "a stiff 45-degree pose"; later rounds
   changed it, not re-checked against footage since).
4. **Likely cause:** Local to the jump, but it shares M1's root (the
   controller's constants are invented).
5. **Evidence:** Footage: jump arcs (measurable from frames against known
   object sizes). Data: unidentified `_HumanPhysics` fields. 360 code: the
   character controller's jump.
6. **Correction:** Establish jump speed and gravity from the controller
   code (M1's research), then drop the pinned pelvis if the original lets
   the clip move it.
7. **Verification:** Apex height and airtime of a standing and a running
   jump, before and after, against footage measurements.
8. **Risk:** Low to medium; autoplay's hop helpers depend on jump height.

### A6

**Ragdolls** · Animation/movement · P2

1. **Original:** *Unknown.* Collision data (`PHY2`) has body records with
   mass and inertia; whether characters carry an authored ragdoll (bodies,
   joint limits) is not established.
2. **Ours:** A ragdoll is built from the skeleton's bones with sizes
   clamped to 0.07 to 0.7 m (invented, `crates/m2-engine/src/physics.rs:1260`)
   and a joint slack of 2 cm (`:947`). How death and knock-down hand over
   from animation is our own.
3. **Difference:** Playtest 4 reported "ragdolls stretch". Limb limits
   and mass distribution differ from the original by construction.
4. **Likely cause:** No ragdoll data read.
5. **Evidence:** Data: search the character models' `PHY2` and other
   asset types for ragdoll bodies and constraints. 360 code: ragdoll setup
   and the switch from animation to physics.
6. **Correction:** Research round first. If authored data exists, build
   ragdolls from it.
7. **Verification:** Ragdoll test scene: drop from 2 m and a blast at 3 m;
   report maximum joint separation and settle time per hero.
8. **Risk:** Medium; deaths are frequent.

### A7

**Root motion vs. code-driven movement** · Unknown pending investigation ·
P2

1. **Original:** *Unknown.* Locomotion clips carry their travel on the
   root track (from samples). Whether the game moves the character by the
   clip (root motion), by code, or each in different states (hijacks,
   vaults, climb-outs, melee) is not established.
2. **Ours:** Root motion is always stripped horizontally and code moves
   the character (`Pose::strip_root_motion`,
   `crates/m2-engine/src/anim.rs:236`); hijacks place clips from the
   vehicle's origin with per-vehicle lifts fitted to footage
   (`crates/m2-alpha/src/hijack.rs` module docs).
3. **Difference:** Unknown until the original is established. Candidate
   symptoms: sliding during turn-in-place and stops; per-vehicle hijack
   offsets.
4. **Likely cause:** Shared, if the original uses root motion in some
   states.
5. **Evidence:** 360 code: where the character's position is advanced
   and whether it reads the root track.
6. **Correction:** Decide after the research; no change before.
7. **Verification:** For each state found to use root motion, root
   displacement vs. character displacement over the clip, error in cm.
8. **Risk:** High if changed broadly; limit to the states the code shows.

### A8

**Decoder residuals** · Visual discrepancy · P3

1. **Original:** *Recovered in part.* Havok 5.5 clips decode for all 4,232
   clips within 7.6e-6 of an independent decoder (PR #8).
2. **Ours:** 14 clips show large steps at wavelet block seams; 10 clips
   name nodes no model has; clip header bytes 0x14 to 0x23 and the `trnm`
   layout signature are unknown (project notes, animation format).
3. **Difference:** A visible pop on those 14 clips, if they are played.
4. **Likely cause:** A decoding detail at block boundaries, or the data
   really steps there.
5. **Evidence:** Data; 360 code: the wavelet decompressor.
6. **Correction:** Compare against the 360 decoder's seam handling.
7. **Verification:** Seam-step metric per clip (largest per-frame
   rotation change at a seam vs. inside a block), before and after.
8. **Risk:** Low; the decoder has a test corpus.

## Character movement

### M1

**Instant starts and stops, full air control** · Animation/movement · P1

1. **Original:** *Unknown.* Footage would show whether the heroes ease
   into and out of a run; nothing has been measured.
2. **Ours:** The wanted horizontal velocity is direction times gait speed
   (`crates/m2-alpha/src/player.rs:824` to `:840`), and
   `Physics::move_character` applies it as given every tick, airborne too
   (`crates/m2-engine/src/physics.rs:2999`). The controller is Rapier's
   kinematic capsule with a 0.5 m step and a 50 degree slope limit, both
   invented (`physics.rs:17`, `:39`, `:41`); turning is 10 rad/s, invented
   (`player.rs:37`).
3. **Difference:** Every start, stop and direction change is immediate;
   the character can steer freely in mid-air. Animation crossfades (A1)
   hide part of this and add sliding.
4. **Likely cause:** Shared. The character controller is a conventional
   one, not the original's.
5. **Evidence:** 360 code: the player character's movement update (find it
   through the readers of `_HumanPhysics`, `b5353104` / `5cc8c4d9` /
   `6167d5fc`). Footage: speed over time at the start and end of a run
   (track a hero against the ground).
6. **Correction:** Research round: recover acceleration, deceleration,
   turn rate, air control, step and slope rules. Then reproduce them in
   the controller, keeping Rapier only for the shape cast.
7. **Verification:** Deterministic controller tests on flat ground and a
   30-degree slope: time to top speed, stopping distance, turn time for
   180 degrees, airborne lateral change. Compare with footage-derived
   numbers or an emulator of the recovered code.
8. **Risk:** High. Autoplay, every mission's on-foot route, the climb
   driver (PmcCon004) and co-op depend on it. Re-run the autoplay contract
   set after the change.

### M2

**Gait speeds** · Behavioral discrepancy · P1

1. **Original:** *Inferred.* `_HumanPhysics` has three fields read as
   walk, run and sprint (`b5353104`, `5cc8c4d9`, `6167d5fc`; meaning
   inferred, heroes notes).
2. **Ours:** Jennifer walks 1.4, runs 5.0, sprints 7.2 m/s, values chosen
   inside the measured speed bands of the game's clips
   (`crates/m2-alpha/src/player.rs:20` to `:25`); other heroes are scaled
   by their template's speeds against hers.
3. **Difference:** Possibly a different pace from the original; feeds A2's
   sliding.
4. **Likely cause:** Template field semantics not confirmed; absolute
   values not taken from the template.
5. **Evidence:** Data: the field values per hero. 360 code: what reads
   those fields. Footage: distance over time for a run and a sprint.
6. **Correction:** Confirm the fields and use their values directly.
7. **Verification:** Unit test the template read; footage pace
   measurement before and after (m/s over 5 s).
8. **Risk:** Medium; route timings in autoplay.

### M3

**On-foot camera** · Visual discrepancy · P1

1. **Original:** *Unknown.* Vehicles read `CameraOffset` from their
   templates; whether the on-foot camera has a template is not known.
2. **Ours:** Eye height 1.6 m, distance 4.0 m, closest 0.6 m, shoulder
   offsets (`crates/m2-alpha/src/player.rs:65` to `:76`,
   `crates/m2-alpha/src/game.rs:268` to `:277`), not marked with a source.
3. **Difference:** Framing of every on-foot frame; not measured.
4. **Likely cause:** Values chosen by eye.
5. **Evidence:** Footage: the hero's screen height and position at rest
   and aiming, at a known FOV. Data: a camera template for people, if any.
6. **Correction:** Fit framing to footage (Approximated) unless the data
   or code gives the rig.
7. **Verification:** Screenshot at a fixed spot, pose and resolution vs. a
   footage frame: hero's head and feet screen positions, error in percent
   of screen height.
8. **Risk:** Low; camera-only.

### M4

**Swimming** · Animation/movement · P3

Swim start and stop depths, float height and speed are invented
(`crates/m2-alpha/src/player.rs:49` to `:62`). Evidence: footage of a swim.
Correction and test: depth at which she starts to swim, and swim speed,
against footage. Risk: low.

## Physics

### P1

**Vehicle handling model** · Behavioral discrepancy · P1

1. **Original:** *Inferred.* The vehicle templates carry fields whose
   names match a full engine, gearbox, brake, steering, suspension and
   tire-friction model (`EngineTorque`, `GearOptRPM`,
   `TorqueFactorAtMinRPM`, `ClutchDelayTime`, `SteerMaxAngle`,
   `FrontWheelSuspStrength`, `FrontWheelFrictionSide`, `FrontBrake`,
   `HandbrakeInertia`, ...; read in `crates/m2-engine/src/tuning.rs`). The
   collision data is Havok 5.5, and these names fit Havok's vehicle kit.
   That the game runs that kit is an inference to confirm in the 360 code.
2. **Ours:** Rapier's `DynamicRayCastVehicleController`
   (`crates/m2-engine/src/physics.rs:1840`) driven by those fields under
   inferred meanings, with invented forms and fallbacks: top-speed easing,
   ground drag, climb help on hills, track pull, parked hold
   (`physics.rs:879` to `:1035`).
3. **Difference:** Playtests have asked about speed ("vehicles feel
   slower"), hill climbing and bikes on their side. Each was tuned
   separately.
4. **Likely cause:** Shared. The handling model is a conventional one fed
   the original's parameters, so its response cannot match.
5. **Evidence:** Data: all template fields (have). 360 code: the vehicle
   step, found from the readers of these fields' hashes. Footage: the
   tank, jeep and boat clips already used (`gameplay/tank-vs-video.md`).
6. **Correction:** Research first (stage F5): recover the per-step order
   (engine, transmission, brakes, steering, suspension, friction,
   aerodynamics), equations and constants; write a small emulator of the
   recovered code, as was done for particle forces. Reimplement
   independently; keep Rapier for the chassis body and collision only.
7. **Verification:** Deterministic tests per vehicle class: 0 to top
   speed time, top speed, braking distance from 20 m/s, turn radius at
   10 m/s, suspension settle, compared numerically with the emulator
   (max absolute and relative error, accumulated over 600 steps). The
   existing examples (`drivetest`, `handlingtest`, `susptest`,
   `steertest`, `biketest`) become the regression harness.
8. **Risk:** Very high. Races, chases, autoplay contract completion and
   co-op depend on handling. Change one vehicle class at a time, re-run
   the race contracts and the vehicle contracts before each merge.

### P2

**Explosion push** · Behavioral discrepancy · P1

1. **Original:** *Data recovered, use unknown.* Explosion templates give
   `Radius`, `Damage`, `MinDamage` and `MaxForce` (read in
   `crates/m2-engine/src/weapons.rs:977`). The particle force kinds were
   recovered from the 360 code (PR #161); the rigid-body push has not been
   traced.
2. **Ours:** A wrecked vehicle is thrown with a fixed velocity change of
   4 m/s outward and 6 m/s up times its mass, only when the blast wrecks
   it (`crates/m2-alpha/src/combat.rs:4486`); people get a fixed push
   (`BLAST_PUSH`, `combat.rs:506`). `MaxForce` is read and not used. Blast
   damage to vehicles uses its own invented falloff (`BLAST_ON_VEHICLE`,
   `combat.rs:538`).
3. **Difference:** Every explosion throws things the same way regardless
   of explosive, distance and mass; unwrecked vehicles are not pushed.
4. **Likely cause:** Shared; the force model was never established.
5. **Evidence:** 360 code: the explosion's effect on rigid bodies
   (follow the `MaxForce` field's readers; check whether it shares the
   force-kind code recovered for particles). Data: per-explosive values.
6. **Correction:** Recover the falloff and mass dependence; apply it to
   vehicles, props, ragdolls and debris through one function.
7. **Verification:** Deterministic test: a 2,000 kg and a 40,000 kg body
   at 2, 5 and 10 m from a grenade, an RPG and the satellite bomb; final
   velocity vs. the emulator. Footage sanity check on the gate and car
   explosions already catalogued.
8. **Risk:** Medium to high: destruction, wreck placement and missions
   that need a vehicle intact.

### P3

**Simulation rate and step order** · Unknown pending investigation · P2

1. **Original:** *Partly recovered:* particles step on a 60 Hz clock
   (from the 360 code, PR #161). The physics step rate, substeps and
   the order of game logic, physics and animation in a frame are not
   established.
2. **Ours:** One 60 Hz fixed tick (`crates/m2-alpha/src/main.rs:129`)
   with Rapier stepped once per tick.
3. **Difference:** None seen; matters for numerical comparison in P1, P2
   and M1.
4. **Likely cause:** Not established.
5. **Evidence:** 360 code: the main loop and the physics world's step
   call.
6. **Correction:** Record the original's rate and order; adapt only if
   they differ.
7. **Verification:** A documented frame-order table; emulator tests run
   at the original rate.
8. **Risk:** Low to establish; high to change.

### P4

**Helicopters, boats, planes and bikes** · Behavioral discrepancy · P2

Each reads its template block (`_HelicopterPhysics`, `_BoatPhysics`,
bike fields) but shapes the response with invented easing:
`YAW_SHARE`, `FLY_EASE`, `CLIMB_EASE`, `LEAN_EASE`, `PLANE_RISE`,
`PLANE_TRIM`, `BIKE_LEAN` (`crates/m2-engine/src/physics.rs:979` to
`:1035`). Same method and risk as P1, one vehicle kind at a time, after
P1 establishes the shared parts. Evidence: 360 code; footage of the
Avenger and Osprey (`falBXvRXBAU`) and boat rides.

### P5

**Collision materials** · Behavioral discrepancy · P3

`PHY2` triangles carry no surface flags (0 on all 1,040,652); shapes carry
a collision class (`crates/m2-assets/src/physics.rs` module docs).
Friction and restitution are Rapier defaults or invented per use
(`WRECK_FRICTION`, `DEBRIS_FRICTION`). Evidence: the terrain and model
material data, 360 code. Test: slide distance of a wreck and a crate on
road, dirt and grass.

### P6

**Projectiles** · Behavioral discrepancy · P3

Speeds, gravity scale, damage and reach come from the templates
(`crates/m2-engine/src/weapons.rs`). The homing turn law and the lock
timing are ours (`crates/m2-alpha/src/combat.rs:4292`); drag and
laser-designation behavior are not established. Evidence: 360 code for
the projectile controller. Test: flight path of a homing rocket against a
target crossing at 20 m/s vs. an emulator.

## UI

### U1

**Flash renderer coverage** · Visual discrepancy · P1

1. **Original:** *Recovered (format).* The menus, pause menu, PDA, loading
   screen and HUD pop-ups are Scaleform movies; the SWF features they use
   are in the movies themselves.
2. **Ours:** Gradients are drawn in their first color, morph shapes are
   not drawn, `PlaceObject3` filters (glow, drop shadow, blur) are ignored,
   and of the blend modes only multiply is drawn
   (`crates/m2-assets/src/flash/mod.rs:26`, `:265`; the module comment
   predates text and ActionScript support and should be refreshed).
3. **Difference:** Wherever the movies use those features, colors,
   highlights and selection glows differ on every screen that uses them.
4. **Likely cause:** Shared; one renderer serves every movie.
5. **Evidence:** Data: a census of the movies (count of gradient fills,
   filters, blend modes and morph shapes per movie). Footage: menu and
   pause frames.
6. **Correction:** Run the census first. Implement in order of how many
   on-screen elements each feature affects (gradients are expected first).
7. **Verification:** Census numbers per movie (features drawn / used),
   before and after. Fixed-resolution screenshots of the main menu, pause
   menu and PDA against footage frames: per-region color difference.
8. **Risk:** Low to medium. A renderer-level change affects every movie;
   check every movie's screenshot, not only the one fixed.

### U2

**Prompts and loading lines in our own text** · Visual discrepancy · P1

1. **Original:** *Partly recovered.* The game shows its own prompts (for
   example `Hijack %s`, its button glyphs, and the hint text on the loading
   screen).
2. **Ours:** "What the player can do next" and loading lines are drawn in
   egui's bundled fonts (`crates/m2-alpha/src/ui.rs:5` to `:8`).
3. **Difference:** A different font, size and position every time a prompt
   shows.
4. **Likely cause:** The game's prompt widget and its event are not wired
   for most actions.
5. **Evidence:** Data: the HUD layout's prompt widgets and the text
   table's prompt strings. Footage: a prompt's position and size.
6. **Correction:** Route prompts through the game's own widget with the
   text table's strings; keep egui only for developer overlays.
7. **Verification:** List of prompts drawn by egui vs. by the game's
   widget, before and after; screenshot of each prompt vs. footage.
8. **Risk:** Low; autoplay reads prompt state, not pixels (check).

### U3

**Aspect ratios and resolutions** · Visual discrepancy · P2

Full-screen Flash movies are fitted with Scaleform's "show all" mode
(`crates/m2-alpha/src/gamehud.rs:4739`); native HUD widgets use the
layout's anchor correction (`crates/m2-game/src/gui.rs`, `Correction`).
Matches to footage were made at single resolutions (HUD round 2:
health ±1 px, weapon box ±3 px). Test plan: the same screens at 1280×720,
1920×1080, 2560×1440, 1920×1200 and 1024×768, with positions reported
in virtual units; every element must hold its anchor's fraction.
Evidence: footage at 16:9 and any 4:3 footage. Risk: low.

### U4

**Native HUD animation** · Visual discrepancy · P2

`AnimateSprite`, `AnimateText` and clock wipes were listed as gaps in the
HUD runtime notes; the PDA was later drawn but the module comment still
lists it as missing (`crates/m2-alpha/src/gamehud.rs:16`). Next step:
list the natives still unimplemented from `Screen::unknown` in a
full-mission run and mark each as used or unused by the HUD scripts.

### U5

**PDA input** · Functional bug · P2

Button presses and mouse clicks do not reach the PDA (audit 2026-10-21).
Already in the project to-do queue; listed here for completeness.

### U6

**Flash text details** · Visual discrepancy · P3

Text layout handles alignment, margins, leading and word wrap; HTML is
reduced to `<font color>`, and kerning tables are not applied
(`crates/m2-assets/src/flash/text.rs`). Census of which text fields use
other HTML tags or kerning first.

## Closed

None yet.
