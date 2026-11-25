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
  to come (What's New, Achievements, Mods, Gallery) and Settings.
- **Header:** the game's name and the tabs: Play, Saves, Options, and
  Patch Notes (to come).
- **Play:** the banner (steps on every 12 s; arrows and dots step it by
  hand) over the bar: the save picker on the left (Continue with the
  newest save, any other save, or a new game), Play in the middle, and
  who plays with what is happening on the right.
- **Saves:** New Game with the hero to start as, then every slot (the
  autosave and saves 1 to 20) with who, cash, fuel and when it was last
  written, and Play, Copy (into the first free slot) and Delete (asks
  once).
- **Options:** the game window (size, windowed or fullscreen), skipping
  the EA and Pandemic movies, keeping the launcher open, and the install,
  program and save folders. Kept in `m2-launcher.toml` beside it.

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

The launcher reads the files `m2-alpha` writes (`m2-alpha-autosave.toml`,
`m2-alpha-save<n>.toml`, beside its settings file). That file format is
the port's own: the contracts done, the factions and the stockpile in it
are the game scripts' own save data, but the wrapper and the world state
beside it are not the 360's save record (traced in
`m2-game/src/profile.rs`, not yet written). So play time, last contract
and percent complete are not shown; a faithful save port would add them.

## To come

Each fits as a sidebar page or a tab:

- **What's New / Patch Notes:** the merged pull requests' playtest notes.
- **Play builds:** pick main or a test build to start.
- **Mods:** a mod manager for `m2-modapi`.
- **Bug report:** bundles `last-run.log` and the build's details.
- **Gallery:** the game's movies and music, from the install.
- **Achievements:** the game's single-player achievements, from its own
  list.
