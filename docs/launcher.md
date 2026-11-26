# The launcher (`m2-launcher`)

A small native program that sits beside `m2-alpha.exe`, lists the save
slots and starts the game. Build it with
`cargo build --release -p m2-launcher` (add `--features windowed` for a
build that opens no console window) and put `m2-launcher.exe` in the same
folder as `m2-alpha.exe`.

Everything it shows from the game is read from the player's own install
at run time (the same `m2-assets` readers the game uses): the Scaleform
fonts (Gunplay for headings, Franklin Gothic Medium for body text) and the
banner pictures (the main menu's, a loading picture, contract briefings,
points of interest). Nothing from the game is committed or cached on disk.

## Layout

- **Sidebar:** who is playing, the game (Mercenaries 2), the pages still
  to come (Mods, Gallery) and Settings.
- **Header:** the game's name and the tabs: Play, Saves, Stats,
  Achievements, Options, and Patch Notes (to come).
- **Play:** the banner (steps on every 12 s; arrows and dots step it by
  hand) over the bar: the save picker on the left (Continue with the
  newest save, any other save, or a new game), Play in the middle, and
  who plays with what is happening on the right.
- **Saves:** New Game with the hero to start as, then every slot (the
  autosave and saves 1 to 20) with who, cash, fuel and when it was last
  written, and Play, Copy (into the first free slot) and Delete (asks
  once).
- **Stats:** two parts, kept apart. **All saves** (marked
  *launcher-tracked*): the total time the game ran, the sessions, the
  longest one and when it was last played, which the launcher counts
  itself in `m2-launcher-stats.toml` beside it (a session runs from Play
  to the game's exit; closing the launcher meanwhile only hides it, so
  the session is still counted). **Each save**: its time played, percent
  done, cash and fuel from the 360 record, and from the scripts' save data
  packed in it the contracts finished and each faction's standing with the
  PMC (Hostile, Neutral or Friendly, as the game's scripts grade it). A
  record stamped 0:00:00 (saved without the pause menu opening, which the
  360 never does) shows the scripts' own clock instead, marked `*`.
- **Achievements:** the game's single-player achievements (36), read at
  start from the scripts' own list in the install (`MrxAchievements`), for
  the save picked at the top. The 360 keeps earned achievements in the
  player's profile, not in saves, so a save can show only those whose
  condition it keeps: the five faction ones (standing at 100), the first
  contract's, the PMC hunting contracts' and the resource counters'. Each
  of those reads Earned or Not yet (in this save); the rest read Not
  tracked, with why. The names and gamerscore are the 360's, from public
  achievement lists matched to the ids by hand (the PC install has none);
  two ids that match no listed achievement keep their id's words. Each
  line of what earns it is the launcher's own wording.
- **Options:** the game window (size, windowed or fullscreen), skipping
  the EA and Pandemic movies, keeping the launcher open, and the install,
  program and save folders. Kept in `m2-launcher.toml` beside it.

## Updates

Each merge to `main` is published by `.github/workflows/release.yml` as a
pre-release `build-<date>-<sha7>`: `m2-alpha.exe`, `m2-launcher.exe`
(windowed), `manifest.toml` (the commit, and each file's size and
SHA-256) and `SHA256SUMS`. Nothing else: both programs read the game's
files from the install, and the workflow refuses an embedded icon or any
other file. Pre-releases leave the beta's `releases/latest` link alone,
and old ones stay up.

A launcher built by that workflow knows its commit and the repository.
On open it asks the public GitHub API for the releases (one request, no
sign-in, a 10 s limit; offline it just says so) and shows **Update
available** in the header when the newest build is another commit.
Update (or **Update automatically when the launcher opens** in Options,
off by default) downloads the files into `update\staged\`, checks each
against the manifest, moves the current ones to `update\previous\` and the
new ones into place. The running launcher is renamed aside like the rest
(Windows lets a running program be renamed, not overwritten), and the new
one runs from **Restart**. **Roll back** swaps `update\previous\` back
(and again, to go forward). Only those two programs are ever moved: the
saves and settings beside them are not touched. A local `cargo build`
knows neither, so it never checks.

## Starting the game

Play runs `m2-alpha` in its own folder with `--size`, `--display`,
`--to-front-end` (when the movies are skipped), `--install` (only when
the launcher was told where the game is), and then:

- a save: `--load-slot <n>`; the game's front end comes up and does its
  own Load Game of that slot at once;
- a new game: `--hero <name> --new-game`; the front end does its own New
  Game as that hero at once.

Both are the whole game's launch (as `--alpha`), and quitting to the
front end later does not repeat them.

## Saves

The launcher reads the files `m2-alpha` writes (`m2-alpha-autosave.sav`,
`m2-alpha-save<n>.sav`, beside its settings file). Each is one 360 save
record (`m2-game/src/save_record.rs`): the time played, cash, fuel, last
contract, percent done, character and time stamp the 360's save list
shows, and the scripts' save data, and nothing else: like the 360, a save
holds no place, health, weapons, vehicles or dead (loading puts her back
at the scripts' start). A stale `.world.toml` left by earlier builds is
not copied and is deleted with its slot. Old `.toml` saves still list,
without play time, contract or percent.

## To come

Each fits as a sidebar page or a tab:

- **Mods:** a mod manager for `m2-modapi`.
- **Bug report:** bundles `last-run.log` and the build's details.
- **Gallery:** the game's movies and music, from the install.

## What's New

The **WHAT'S NEW** tab (sidebar and header) lists the merged pull requests,
newest first, by the day they were merged: number, title and a one-line
summary. It reads `docs/changelog.toml`, built into the launcher, so it works
offline. That file is made from the pull requests' public titles and
descriptions by `tools/changelog/changelog.py` (`gh pr list`; attribution
lines, headings, check details, paths, links and the names given in
`CHANGELOG_PRIVATE` are left out). Regenerate it before a release:

```powershell
$env:CHANGELOG_PRIVATE = '<names to leave out>'; python -I tools\changelog\changelog.py
```

## Window size on a first run

With no settings file yet, the game's window size starts at the primary
monitor's size in pixels (even, at least 640x360) instead of a fixed
1920x1080; once the settings are saved the chosen size is kept
(invented: the game's own launcher had no such default).

## Picking the build to play

A chip at the top right of the Play tab's banner names the build Play
starts (`BUILD  <tag> - <date>`). Its list offers:

- the build installed beside the launcher (the default);
- the build kept for rolling back (`update\previous\`);
- every build kept under `builds\<tag>\` beside the launcher, newest first;
- the latest release, when it is none of these. Picking it downloads its
  `m2-alpha.exe` into `builds\<tag>\`, checked against the release's
  manifest (size and SHA-256). The launcher itself is not replaced.

The pick is pinned in the launcher's settings (`build = "<tag>"`). A kept
build runs from the launcher's folder, so it reads the same settings and
saves. If a pinned build's program is no longer there, Play starts the
installed build and the status line says so.

Builds are never deleted. Before an update replaces `update\previous\`, the
build there is copied into `builds\`; old builds pile up there until
someone deletes them by hand. Labels are the release tags and build dates:
our own text. Everything here is invented: the game had no such picker.
