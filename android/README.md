# Mercenaries 2 Beta on Android

The desktop game (`crates/m2-alpha`) built as an Android app ("Mercenaries 2
Beta", package `com.bstoney.m2alpha`) for an arm64 phone or tablet with
Vulkan (Android 8.0 / API 26 or later). It plays with a Bluetooth keyboard
and mouse (Samsung DeX works well), a game controller, or the on-screen
touch controls.

The game's files are never in the APK: the app reads your own copy from a
folder on the phone.

## What differs from the PC

- **Movies and the menu music** (boot logos, the front end's backdrop,
  cut-scenes) play only when the APK was built with its small FFmpeg
  (`libffmpeg.so`, see [Movies](#movies-the-menus-backdrop-and-music)). An
  APK built without it skips them; `m2-alpha.log` says which
  (`movies: ffmpeg ...` or `movies: this APK carries no ffmpeg`).
- **Background:** sending the app to the background pauses play and the
  sound; coming back makes a new surface and play goes on. If Android gives
  no surface back, the app closes (the log says
  `back from the background: no surface`).
- **Defaults:** frames are capped at 30 a second and the game renders at
  0.6 of the screen's resolution (`max_fps` and `render_scale` in the
  settings file; the PC defaults are uncapped and 1.0).
- **Mouse:** click in the game to capture it, as on the PC; Escape lets it
  go. While captured the cursor is hidden and the mouse turns the camera.
- **Controller:** an Xbox pad over Bluetooth (or any pad the phone takes as
  a gamepad) uses the PC's pad layout (`crates/m2-alpha/src/pad.rs`).
- **Touch:** without a controller, an on-screen pad (`touch.rs`): a stick
  on the left, drag elsewhere to look, and buttons named as the 360 pad's
  (A B X Y in their colours, LB/RB, LT/RT, LS, the d-pad, START, BACK).
  The touch layout is invented; the original game had none.

Everything else (the mission, HUD, saves, settings) is the PC build's.

## Copying the game to the phone

Make a folder named `Mercenaries2Beta` holding the APK and your install's
`data` folder (`<install dir>\data`; its `Movies` folder can be left out,
which also skips the movies). Then:

1. Connect the phone by USB, allow file transfer, and drag the
   `Mercenaries2Beta` folder onto the phone's `Internal storage`.
2. On the phone, open it in My Files and tap the APK to install it (allow
   installs from unknown sources when asked).
3. Open the app. The first time it opens the "All files access" page:
   switch it on, then open the app again.

The app looks for the game, in order, in `Mercenaries2Beta`, the older
`Mercenaries2Alpha`, the top of internal storage, `Download/Mercenaries2Beta`,
`Download/Mercenaries2Alpha`, `Download`, and last its own folder
`Android/data/com.bstoney.m2alpha/files` (which needs no permission, but is
deleted when the app is uninstalled).

**Logs and files:** `Download/m2-alpha.log` holds what the game reported
this run and `Download/m2-alpha.prev.log` the run before (send these when
something goes wrong). The game's folder holds `m2-alpha-crash.log`, the
settings file, saves and the catalogue cache. Arguments the PC build takes
on its command line (`--no-boot`, `--spawn x,z`, ...) can be written into
`m2-alpha-args.txt` there.

**Updating:** install a new APK over the old one. Android refuses an update
signed with a different key ("App not installed"): when the signing key has
changed (for example, the build machine made a new one), uninstall the old
app first. Saves in the `Mercenaries2Beta` folder are kept; anything in the
app's own `Android/data` folder is not.

## Building the APK (on Windows)

One-time setup:

1. The Android SDK command-line tools, then with its `sdkmanager`:
   `platform-tools`, `platforms;android-34`, `build-tools;34.0.0` and an NDK
   (`ndk;27.2.12479018` was used). Set `ANDROID_HOME` to the SDK and
   `ANDROID_NDK_ROOT` to the NDK.
2. A JDK (for `keytool`), on `PATH` or as `JAVA_HOME`.
3. `cargo install cargo-apk` (the script adds the `aarch64-linux-android`
   Rust target itself).
4. Optional, for movies: build FFmpeg once (next section).

Then run `android\build-apk.ps1` (`-Install <install dir>`, or set
`M2_INSTALL`, when the game is not in the default EA Games folder). It:

- copies winit 0.30.13 from the cargo registry into `android\target\winit`
  and applies `winit-0.30.13-input.patch` (upstream winit reports an
  Android mouse and a controller's sticks as touches);
- makes a signing key at `%USERPROFILE%\.android\m2-alpha.keystore` if
  there is none. **Back this file up**: a new key means every phone must
  uninstall before the next APK installs;
- scales the install's `mercs2.ico` into the launcher icons under
  `android\target\res` (without it the placeholder in `res\` is kept);
- warns when `android\target\runtime-libs\arm64-v8a\libffmpeg.so` is
  missing (the APK then skips movies and the menu music);
- builds `android\target\release\apk\Mercenaries2Beta.apk`.

`android\` is its own Cargo workspace so the desktop builds and CI never
compile it. Its library is `crates\m2-alpha\src\main.rs`; the Android entry
point is `crates\m2-alpha\src\android.rs`. When m2-alpha gains a
dependency, add it to `android\Cargo.toml` too, or the APK build fails.

## Movies (the menu's backdrop and music)

The game's movies are Bink files, played through a small FFmpeg the APK
carries as `lib/arm64-v8a/libffmpeg.so` (Bink in, raw pictures and sound
out; LGPL parts only). Build it once from an FFmpeg 7.1.1 release tarball,
from Git Bash with `ANDROID_NDK_ROOT` set, before `build-apk.ps1`:

    sh android/build-ffmpeg.sh <ffmpeg-7.1.1 source folder>

It lands in `android/target/runtime-libs/arm64-v8a/`, which a clean of
`android\target` deletes; rebuild it after one.
