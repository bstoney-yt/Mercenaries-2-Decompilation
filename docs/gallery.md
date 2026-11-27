# Cut-scene and music gallery: design note (not built yet)

Status: a proposal for review. No code yet. Everything is read from the
player's own install when the page opens. Nothing is extracted into the
repository, cached as files, uploaded or shared.

## What exists today

- **Movies.** `m2-alpha`'s `movie.rs` plays `data\Movies\*.bik` (Bink 1)
  through a separate `ffmpeg` process, found as `M2_FFMPEG`, beside the
  exe, or on `PATH`. The frames and sound are piped back and never
  written. Sound plays at the dialog volume on `Category::Movie`.
  `movie::Header::parse` gives the size and length from the file's header.
- **Music.** `m2-assets`'s `sound.rs` reads the streamed banks:
  `StreamIndex` points into `data\Audios\music.pws`, and `Stream` gives the
  hash, channels, rate and length. `decode_xbox_adpcm` decodes them.
  `m2-alpha`'s sound engine plays the scripts' music cues
  (`mu_fac_pmc_explore_01`...) by the cue table: name hash to stream
  record, with looping.
- **Names.** Movies have file names. Music streams have only hashes; their
  cue names (`mu_*`) come from the cue table and the scripts' strings, as
  the game reads them.

## The page

A **Gallery** page in the launcher (the sidebar's "soon" item), with two
lists:

- **Cut-scenes:** each `data\Movies\*.bik`, with its length from the
  header. The list shows our own label for each file: the file name
  tidied, and later the contract it belongs to where the scripts say.
  **Play** opens a launcher viewport (or full screen) that decodes with the
  same ffmpeg pipe. There is no seeking in version 1, only stop.
  "Unlocked" (seen in a save) can follow later: version 1 lists them all.
- **Music:** each music cue, by its `mu_*` name, read from the cue table
  in the install at run time, with its faction and length. **Play** and
  **Stop** decode the stream in memory (Xbox ADPCM) and play it through the
  launcher's own small output.

The labels are our own words or the game's identifiers as read at run
time. No game text or pictures are committed. The page says the media is
the player's own install's.

## How it is built

1. **Share the readers.** Move the movie listing and the ffmpeg decode
   (today in `m2-alpha`'s `movie.rs`) and the music listing and decode
   (cue table plus `StreamIndex` plus ADPCM) into a library both programs
   can use. They could go in `m2-engine` (which already holds `Install`)
   or a small new crate. No behaviour changes.
2. **Lists:** pure functions over the install, with tests on synthetic
   headers, cue tables and streams. No game data in the tests.
3. **Sound in the launcher.** The launcher has no audio output today. Add
   `cpal` (as `m2-alpha` uses on Windows) behind the gallery, opened only
   while something plays.
4. **The page:** two lists, a player strip (playing, time, Stop), and a
   message when ffmpeg or the install is missing.
5. **Movies in a viewport:** decoded frames go to an egui texture. If
   that proves too slow at 1280x720, start `m2-alpha --movie <name>`
   (a small new flag) instead.

Steps 1 and 2 are pure code, and step 4 has a launcher-screenshot check.
Steps 3 and 5 need a person's ears and eyes, or a game-check thread's
offscreen run.

## Decisions (2026-11-26)

Brendan left the four questions below to the project; these are the
answers it chose.

1. **List everything.** Every cut-scene and every music cue is listed.
   Those the current save hasn't seen are shown greyed and locked, read
   from the save data the launcher already reads. Where a save has no
   seen-flag for an item, it is shown unlocked, with a note saying so.
2. **Movies play inside the launcher's viewport.** Movies need a local
   ffmpeg; without one, the page says so plainly and still plays music.
3. **Readable names.** Real game strings read from the install at run
   time are used where they exist. Otherwise a name of our own is shown,
   falling back to the raw file or cue name. Every name of our own, and
   the fallback, is marked as invented.
4. **Music plays without ffmpeg,** through the native Xbox ADPCM decoder
   in `m2-assets`.

The build ships in two PRs: music first, then movies.

## Open questions (answered above)

- Should everything be listed, or only what a save has seen (cut-scenes
  of finished contracts, factions met)? The second needs the save's
  script data, which the Stats page already reads.
- Movies in the launcher, or handed to `m2-alpha` (`--movie`)? The
  launcher is lighter; `m2-alpha` already has the player.
- Should music show raw cue names (`mu_fac_pir_combat_02`) or our own
  readable labels ("Pirates: combat 2")?
- Should the gallery work without ffmpeg (music only), or ask for it
  once?
