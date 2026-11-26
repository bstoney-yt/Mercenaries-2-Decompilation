# Contract tracker

The launcher's **Contracts** tab shows which of the story's 52 contracts
play through in this build. It is also a public roadmap, so it holds only
our own text: contract ids, a status, a short note, issue numbers and the
date of the evidence. No game text, pictures or 360 data.

The data is `docs/contracts-status.toml`, built into the launcher. The
`m2-tracker` crate reads, checks and updates it.

## Statuses

| status | means | needs |
|---|---|---|
| `playable` | a recorded run completed it with nothing flagged (and no test-shortcut hops) | evidence and a checked date |
| `playable-with-gaps` | it completes, but with hops, flagged faults or known gaps | evidence, a date and a note |
| `not-yet` | a recorded run did not get through it | evidence, a date and a note |
| `unknown` | no recorded result | nothing |

The `evidence` field says where a status came from: `autoplay` (a sweep or a
thread's run), `nightly` (the nightly autoplay runner), `playtest` (someone
played it) or `none`. **Nothing is marked playable without a recorded
result.** `m2-tracker check` refuses the file otherwise, and so does the
launcher's test.

## Updating it

From a nightly autoplay night (`night.json`, see docs/nightly-autoplay.md):

```powershell
cargo run -p m2-tracker -- update --night Z:\mercs2\scratch\nightly\<night>\night.json
cargo run -p m2-tracker -- check
```

`update` applies these rules to each run:

| run | status |
|---|---|
| pass with no hops | `playable` |
| pass with hops | `playable-with-gaps` |
| fail that still completed (for example a frame-time cliff) | `playable-with-gaps` |
| fail that didn't complete | `not-yet` |
| skipped or aborted | left alone |

Each updated entry gets the night's date, its head and a note generated
from the run. Issue numbers are kept. An entry marked `locked = true` (a
playtest's word) and an entry with newer evidence are left alone. Commit
the changed file in a PR like any other.

By hand: edit the entry, set `evidence` and `checked`, write the note in
your own words and list the issue or PR numbers, then run `check`.

## The seed (2026-11-26)

These statuses come from runs recorded before the nightly runner existed:

- the contract sweeps of 2026-11-07 to 2026-11-10;
- the known-bugs issue #698;
- docs/fidelity/GAPS.md.

The nightly will move them on.

- **Playable:** OilCon003. Completed with no hops in the 2026-11-07 sweep; not re-run since.
- **Playable with gaps:**
  - VzaCon001: completes; the test bot is slow at the supply crate.
  - PmcCon001: completes; its hops were not recorded.
  - OilCon020: completes only with a hop; #776 since.
- **Not yet:**
  - AllCon001: completed once, then failed twice.
  - AllCon003: the test bot stalls at the AA guns.
  - ChiCon001, ChiCon003, GurCon002, JetCon001: ended without completing in the 2026-11-09 and 2026-11-10 sweeps.
- **Unknown:** the other 42. PmcCon004, PirCon001 and AllCon002 carry notes from GAPS.md on what they depend on.
