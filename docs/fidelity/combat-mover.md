# The 360's combat mover (HumanCombat's move block)

What a soldier does with his feet while he fights, as the 360 program
does it. Traced in the 360 program (`/default.xex`, base `0x82000000`)
for the combat-mover port; every line below is **recovered code** unless
it says **inferred** or **unknown**. Nothing here was checked against
footage yet. Offsets are into the move block (the mind's `+0x130`)
unless they say "mind".

This replaces the alpha's invented fight loop in `npc.rs` (`STEP_GAP`,
`STEP`, `PUSH_TO`, the chase towards the noise and `GIVE_UP`): the 360
has no random 3 to 6 m strafe and no 15 m push. It stands and fires, sits
the fight out crouched or lying, takes a spot from the cover service,
side-steps by animation now and then, throws grenades, and walks to the
target only while it has no shot.

## Where it runs

`HumanCombat` (task descriptor 0x82B79DE0) has three functions:

| Fn | Address | What it does |
|---|---|---|
| start | 0x822B3668 | Resets the move block (0x822C3EB8: state 1, destination = where he stands, speed 0.4), then sets the destination to where he stands and the speed to his `CombatSpeed` (mind `+0x9A8`) |
| think | 0x822B34A8 | Close-quarters branch (0x822B2D30, ported as `ai_close`) when within `r_self + r_target + 1.5` m with nothing between; no path holder (mind `+0x3B8`): stop and fail; else the attack step |
| end | 0x822B3708 | Queues the idle state, ticks the block once (so the old state's exit runs), stops him |

The attack step (0x822B3130), every frame:

1. The target record (mind `+0x7F4`) gone: fail. Its flags with bit 0
   clear and bit 5 set: succeed.
2. The fire controller updates (0x822C95D8 on the mind's `+0x5C` block;
   it sets the shot flags the states read, below).
3. The move block ticks with the target (0x822C8530).
4. He paths to the block's destination (0x822BF7A8) and faces either the
   fire controller's aim point (mind `+0x74`) while aiming, or the
   destination.
5. Not yet within 0.316 m of the destination (squared distance over 0.1,
   0x822CB340): he moves along the path at the block's speed (`+0xC`,
   0x822D19C8), and the dither count runs (0x822BF4D8, ported as
   `ai_dither`); past 8 he marks the spot bad for 60 s and the task
   fails. At it: he stops.
6. Running (the planner keeps the task).

## The block

| Offset | What |
|---|---|
| `+0x00..+0x08` | Destination (where he is told to go; "stand" is his own position) |
| `+0x0C` | Speed: 0.4 after a reset, then `CombatSpeed` (mind `+0x9A8`) or `PursueSpeed` (mind `+0x9A4`) |
| `+0x10..` | The spot the cover service granted (position at `+0x10`; clips at `+0x3C`, `+0x40`, `+0x48`; its `CoverAnimation` at `+0x54`; valid flag `+0x5C` bit 0x80) |
| `+0x60` | The target's perception record (`ai_sense`: its `+0x80` position, `+0x60` ground distance, `+0x8C` age) |
| `+0x64` | The cover entry he stands at (mind `+0x3B8 + 0x3C x` mind `+0x4E8`), or none |
| `+0x68..+0x78` | The current state: enter, exit, update, id, return id |
| `+0x7C..+0x8C` | The queued state, same five words |
| `+0x90` | Stance asked of the body: 0 `stand`, 1 `crouch`, 2 a third (hash 0x5A86B86B; **inferred** lying) |
| `+0x94` | Seconds in this state |
| `+0x98` | Seconds since entry (zeroed on a change, not counted by the tick: **unknown** who counts it) |
| `+0x9C`, `+0xA0` | Two free timers, counted up |
| `+0xAC`, `+0xB0`, `+0xB4`, `+0xC0` | Countdowns: spot check, cover ask, grenade, side-step |
| `+0xB8` | The hold in state 1, counted down by state 1 itself |
| `+0xE4..+0x120` | The four bad spots and their timers (ported as `ai_dither`) |
| `+0x124` | Flags |

The tick (0x822C8530): store the target and cover entry; count the
countdowns down and `+0x94`, `+0x9C`, `+0xA0` up by the frame's step; send
the body the stance message if `+0x90` differs from its stance; run the
current state's update; then, if a state is queued, run the old state's
exit, make the queued one current, zero `+0x94..+0xA0`, run its enter.

A state change (0x822C3FE8, and the same copy inlined everywhere) sets
`+0x90` to 0 (standing) and queues the state with a return id. A state
"returns" by going to its return id, or to a default when it has none.

## Shot flags the states read

From the fire controller (0x822C95D8):

- **Has a shot**: its line to the target came back clear (mind `+0xCC`
  bit 0x80, set from the ray's result) and its verdict (mind `+0x128`
  top byte) is not 4. The verdicts: 0 ready, 4 beyond the gun's reach,
  5 inside its least range, 6 off its arc, 2, 3, 8, 9 **unknown** (9 is
  set by some actions, and grenades are not thrown on it).
- **Firing**: the burst phase (mind `+0x128 & 0xF00000`) is `0x300000`.

## Helpers

| Fn | Name here | What |
|---|---|---|
| 0x822C8280 | busy | He is cowering (mind `+0x10` = `Cower`) or in one of four actions: tell the body to stand or play the action's exit, and say busy |
| 0x822C04A0 | in a one-shot | His current action (mind `+0xC`) is one of 15 one-shot clips |
| 0x822C0740 | target out of his area | His area (mind `+0x274`: centre `+0x7A8`, radius `+0x7BC`, unbounded when the radius is negative or flag `+0x7C4` bit 0x40) does not hold the target |
| 0x822C43B8 | ask for cover | See state 1 |
| 0x822C7CF0 | spot check | Re-checks the granted spot; drops it (bad for 10 s, to state 9) if it no longer serves |
| 0x822C6B78 | grenade gate | See state 14 |
| 0x822C8370 | mark bad | The four-slot bad-spot list (ported) |

## The states

Table at 0x82B77F08, five words each. Constants are the program's.

| Id | Enter / exit / update | What he does |
|---|---|---|
| 0 | none | Idle (the end) |
| 1 | 4AC8 / 4B50 / 4BA8 | **Decide.** Stands; holds `(rand + 1) x 0.5` s (0.5 to 1 s) |
| 2 | 5340 / 5398 / 53F8 | **Fight standing**, or **close in** |
| 3 | 56E0 / 5790 / 57D8 | **To the spot** |
| 4 | 5880 / 58D8 / 5920 | **At the spot** |
| 5 | 5BE0 / 5C38 / 5C80 | Lean out (SideStep cover) |
| 6 | 5D98 / 5DF0 / 5E38 | Fire from the lean |
| 7 | 5F88 / 5FE0 / 6028 | Lean back |
| 8 | 6138 / 61B0 / 6208 | Dive (Dive cover) |
| 9 | 6698 / 66E0 / 6728 | Spot lost |
| 10 | 67A0 / 6808 / 6868 | **Fight crouched** |
| 11 | 6A08 / 6A58 / 6AA0 | **Fight lying** (inferred) |
| 12 | 4DC0 / 4EA0 / 4EE8 | `HumanTakeCover`'s entry (its start 0x822B3970 queues it) |
| 13 | 5078 / 5110 / 51A8 | **Unknown** (an ordered action with three exit clips) |
| 14 | 7208 / 72D0 / 7348 | **Throw a grenade** |
| 15 | 7708 / none / 7730 | **Side-step** |
| 16 | 7CA0 / none / 7CC0 | Passes straight to its return (default 11) |
| 17 | 62E0 / 6330 / 6398 | Pop up and duck (Popup cover) |

All "stand" below means destination = where he stands; the speed is
`CombatSpeed` unless it says otherwise.

### 1, decide

Each frame, with a target and not busy: the hold runs down. Then:

1. Ask for cover (0x822C43B8). Asked only with the service on and fewer
   than 8 asks outstanding, his ask timer `+0xB0` out, his area not
   barring it (mind `+0x274 + 0xC` bit 9), and either a `HumanTakeCover`
   return (13) or the mind's `+0x7F0` bit 4 (**inferred**: may use cover).
   The ask carries his area's centre, a radius of the area's at most
   20 m, the target, and the spots to avoid (live bad spots and
   squadmates, at most 15: ported as `ai_cover::Request`). An answer sets
   `+0xB0` to 5 s; a granted spot is copied into the block.
2. Granted (or a spot already held): to **3**.
3. Waiting on an answer, or flagged mind `+0x70F` bit 3: stay.
4. Else roll `r` in 0..6000:
   - `r < 1000`, the target at least 10 m off, crouching allowed (mind
     `+0x274 + 0xC` bit 3 clear): to **10**;
   - else `r < 1500`, at least 20 m, lying allowed (bit 4 clear): to **11**;
   - else to **2**.

So about 1 in 6 decisions crouch and 1 in 12 lie, at range.

### 2, fight standing / close in

- **Has a shot**: the no-shot timer `+0xA0` is zeroed. Once `+0x9C` has
  passed 0.5 s: the grenade gate (to **14**, return 2), else stand; then
  after more than 5 s in the state with the side-step countdown `+0xC0`
  out: to **15** (return 2); else after `+0x9C` passes 10 s: to **1**.
- **No shot**: once `+0xA0` passes 2 s and the target is inside his
  area: zero `+0x9C`, destination = the target's position, speed =
  `PursueSpeed`, and the first time a callout (hash 0xBF0C1F5D, 50 m).
  Outside his area he stays put.
- Cover asked for and granted: to **3**.

This is the whole "he hears the fight" chase: towards the record's
position, at `PursueSpeed`, only without a shot and only inside his area.

### 10, fight crouched

As 2, but: crouched (`+0x90` = 1) while he has a shot, no side-step and
no grenade; `+0x9C` past 10 s returns (default 1). No shot: the same
close-in.

### 11, fight lying

Stance 2 on entry. Back to **1** after 30 s in the state or with the
target within 10 m. While he has a shot `+0x9C` stays zero; 5 s without
one returns (default 1).

### 3 to 9 and 17, cover

- **3**: destination = the spot, a callout (0xA2097691); at it (0.316 m,
  the spot still good): return (default 4).
- **4**: away from the spot (0.5 m): to **1**. Faces the target. After
  2 s, not in a one-shot, his entry not barred: plays the spot's
  approach clip (`+0x40`) if any; then by the spot's `CoverAnimation`:
  SideStep to **5**, Dive to **8**, Popup to **17**.
- **5 / 6 / 7** (SideStep): lean out (clip `+0x48`), fire from it until
  a burst has run and mind `+0x8C` (**unknown**) is at or under 0 (the
  grenade gate first, to **14** return 7), lean back (clip `+0x3C`), to
  **4** again.
- **8** (Dive): the spot is marked bad for 20 s on entry, the dive clip
  plays, then return (default 2).
- **17** (Popup): four steps: down; up after 3 s (unless the verdict is 9
  or mind `+0x70F` bit 3); fire; down again after the burst. The grenade
  gate between, to **14** return 17. Off the spot (0.45 m): to **9**.
- **9**: `+0xB0` zeroed (he may ask again at once), to **1**.

### 14, throw a grenade

The gate (0x822C6B78): his grenade timer `+0xB4` out, a grenade to throw
(mind `+0x4EC`), the shared pause (a single global) out, the target 10 to
40 m off and on foot or reachable, has a shot, a fresh record (age 0),
not in a one-shot, his area not barring it (`+0x274 + 0xC` bit 12). The
throw speed is `15 + (d - 10) / 30 x 5` m/s (15 to 20), aimed at the
target plus a random offset up to 2 m, solved for a -9.8 m/s2 arc
(0x822CB3D0; the high arc only beyond 4 m of climb and within 3 m).

On entry the shared pause is set to `rand x 7` s. Then: face the throw;
play the throw (0x4872AC15); at the release the grenade goes; at its end
`+0xB4 = 15 + rand x 10` s and the shared pause `5 + rand x 10` s;
return (default 1).

Note: `ai_cover::may_ask` and its `RETRY` treat this gate as the cover
gate. It is the grenade gate; the cover ask is 0x822C43B8. To be
corrected in round (c).

### 15, side-step

On entry he probes 2 m either side of himself (0x8215A770, two rays from
his middle). Both clear: a random side; one clear: that side; neither:
back to **1**. Plays the side's clip (0x94247D15 or 0x0C697AF4), and
`+0xC0 = 10 + rand x 15` s before the next; when it ends, return (default
11 for a left/right play; **unknown** why 11). Another use sets `+0xC0`
to 10 s.

## Give-up and return to post

There is no 20 s give-up in the mover. What ends a fight in the 360:

- The task fails when the target record goes (the record outlives its
  last update as `ai_sense` has it), when it is marked done (step 1), or
  past 8 dithers. The planner then picks again from the goal state
  (`ai_plan`).
- He never chases out of his area (0x822C0740): with the target outside
  it he stands.
- **Unknown**: what takes him back to his post. Not the mover: it is a
  planner goal once `HumanCombat` stops (round (d)).

## Constants

| Value | Where | Use |
|---|---|---|
| 0.5, 1 | 0x820D9EDC, 0x820BAEE8 | Hold `(rand + 1) x 0.5`; 0.5 s with a shot before acting |
| 2 | 0x820C77A4 | No shot this long before closing in; settle at a spot |
| 5 | 0x820BAF84 | Side-step only after this in state 2; ask timer; 11's no-shot leave |
| 10 | 0x820BAEEC | Leave 2 / 10 after this; crouch needs this range; 11 leaves within it; bad spot from the spot check |
| 20 | 0x820BB204 | Lying needs this range; cover ask radius cap; Dive spot bad |
| 30 | 0x820BB1F8 | Longest in 11 |
| 6000, 1000, 1500 | code | The decide roll |
| 10, 40 | 0x820BAEEC, 0x820BAFA8 | Grenade range |
| 15, 10, 5, 7 | 0x820B7B44, 0x820BAEEC, 0x820BAF84, 0x820D329C | Grenade timers |
| 10, 15 | 0x820BAEEC, 0x820B7B44 | Side-step gap `10 + rand x 15` |
| 2 | 0x820C77A4 | Side-step probe |
| 0.1, 0.25, 0.2 | 0x820BB468, 0x820DABDC, 0x820B8054 | Arrival (squared) |
| 60 | 0x82B7D884 | Dither bad spot |

## Left for the next rounds

- (b) Port the block, the tick and states 0, 1, 2, 10, 11 into
  `m2-engine`, and drive `npc.rs` from it in place of `STEP_GAP`, `STEP`,
  `PUSH_TO` and the chase.
- (c) States 3 to 9, 12, 17 with the cover service, 14 with the gate
  renamed, 15.
- (d) What ends the fight and takes him home (task results and the
  planner's goals), in place of `GIVE_UP`.
- **Unknown** still: state 13, the stance-2 name, verdicts 2/3/8/9, who
  counts `+0x98`, the flags at mind `+0x70F`, `+0x7F0` and the area's
  `+0xC`, and whether states 15 and 16 return to 11 by design.
