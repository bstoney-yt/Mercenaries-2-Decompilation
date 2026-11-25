# The physics frame: substeps, order and deferred impulses

What the 360 program does with one game frame's physics, as read from the
code (addresses in the Xbox 360 program, base 0x82000000). It is described in
our own words; no decompiler output is reproduced. Each line has an evidence
label from [README.md](README.md).

## Substeps per frame (recovered code)

- The physics step **0x82152E20** is called from the frame update
  **0x8221E340**, right after the physics manager's update **0x82152B68**.
- With the frame's dt of at least 0.001 s and the substep flag byte
  **0x82B7C99A** set (it is set in the image), the number of substeps is
  `n = floor(dt x 60 + 0.99)`, clamped to **1..3** (constants at 0x820F8BAC
  and 0x82B7D824, floor by 0x8285D5F0). With the flag clear, n is 1.
- Each substep is `dt / n` long (stored at 0x831334A8; n at 0x831334AC).
  The step runs **0x82165A88** n times. That runs the game's per-substep
  physics updates and then Havok's world step (0x82621B38 → **0x82644C38**).
- So the 360 steps physics at about 60 Hz. A 60 fps frame is one substep;
  a slower frame is two or three equal substeps, never more.
- Xenia agrees (phys7b-d): frame dt 0.0472 and 0.0342 s each showed three
  equal drops of gravity, 9.0 x dt / 3. This replaces the earlier record of
  "one Havok step per frame, no substeps".

## Order within a frame (recovered code, one gap)

1. The manager update **0x82152B68**:
   - the rebuild pass **0x82152780** (objects queued for new bodies: old bodies
     torn down, new ones built, saved velocity added back);
   - **0x82155480**;
   - the **action list** (manager +0x388C). Each action's step is called with the frame's dt.
     The death launch (0x8215D978) and the explosion pushes (0x8215C438)
     run here;
   - message queues (0x821544F0) and per-object updates.
2. The physics step **0x82152E20**: the n substeps.

So everything the actions do is decided **before** the frame's first
substep, on the bodies the rebuild pass has just made.

## Deferred impulses (recovered code; the flush point is not yet read)

- An impulse on a rigid body goes through **0x8262AB60**. If the body's
  world is locked (world +0x84 non-zero), it is not applied there. It is
  queued instead as pending op type **0x1A** (0x82621DE0 → 0x82642720 on the
  world's queue at +0x7C).
- The queue is a plain list. Each call appends one 20-byte entry (type, body,
  vector), keeps a reference to the body and counts the entry. It is **not keyed per
  body**: two impulses on one body are two entries, applied in turn.
- Havok's step **0x82644C38** locks the world (+0x84 += 1) and marks the
  queue at its start. Its end, where the world is unlocked and the queue is
  run, is vector code the decompiler can't follow. **Inferred**: queued
  impulses are applied when the world next unlocks, inside a substep. That
  fits Xenia's phys7c/d, where the explosion pushes showed as separate
  level-only changes between substeps, and the world read locked between
  frames.

## The port's frame against the 360's (from the code of both)

- **The 360's frame (0x8221E340):** the manager's actions (rebuild,
  0x82155480, the action list, message queues, per-object +0xCC), then the
  1-3 substeps (0x82152E20), then the rider system (0x82299918) and the rest.
- **The port's frame (`Game::step`):** the player and physics phase
  (`Physics::step_frame`, the same 1-3 substeps of dt/n), then the riders,
  scripts and `Combat::step` (shots, blasts, wrecks, the death launch, the
  explosion pushes).
- **The cyclic order is the same**: actions, substeps, riders, actions,
  substeps, riders... The port's frame boundary falls between the riders
  and the actions instead of before the actions. A sample taken at the end
  of a port frame sees that frame's actions without their gravity. The
  360's sees them with it. Checks that compare per-frame values with Xenia
  account for that (#728's `--wreck-blast-check`).

### Still differs

- **The substeps after a frame's actions use the next frame's dt in the
  port.** The 360 steps frame N's actions with frame N's dt (its
  substeps follow them in the same frame). The port's actions of frame N
  are integrated by frame N+1's `step_frame`, with frame N+1's dt and
  substep count. At a steady frame rate the two are the same. When the
  frame time changes (a slow frame after a fast one), the first substeps
  after an action differ in length and count.

## Consequences for the port (Inferred; not yet changed)

- The port steps physics at a fixed 1/60 s, which is the 360 at 60 fps.
  At lower frame rates the 360 takes up to three substeps a frame. The
  actions run once per frame, before them.
- A check that compares per-frame numbers with Xenia should sample once
  per frame and undo the frame's gravity (9.0 x dt), not a fixed 1/30 s.
