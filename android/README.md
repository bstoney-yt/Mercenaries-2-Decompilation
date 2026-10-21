# Mercenaries 2 alpha on Android

The desktop game (`crates/m2-alpha`) built as an Android app, for a phone or
tablet with a Bluetooth keyboard and mouse (Samsung DeX works well) or a
game controller. It needs Vulkan (any phone from the last several years)
and an arm64 CPU.

The game's files are never in the APK: the app reads your own copy from its
folder on the phone.

## What differs from the PC

- Movies (boot logos and cut-scenes) are skipped: they are decoded by
  `ffmpeg`, which the phone doesn't have.
- Sending the app to the background closes it (the renderer can't move to
  the new window Android gives it on return yet).
- The mouse: click in the game to capture it, as on the PC. Escape lets it
  go. While captured the cursor is hidden and the mouse turns the camera.
- A controller (an Xbox pad over Bluetooth, or any the phone takes as a
  gamepad) plays with the PC's pad layout (see `crates/m2-alpha/src/pad.rs`):
  its buttons arrive as Android's gamepad keys and its sticks, triggers and
  d-pad as axes, which the game turns into the same reading XInput gives on
  the PC.

Everything else (the mission, HUD, saves, settings) is the PC build's.

## Copying the game to the phone

Make a folder named `Mercenaries2Alpha` holding the APK and your install's
`data` folder (`C:\Program Files (x86)\EA Games\Mercenaries 2 World in
Flames\data`; its `Movies` folder can be left out). Then:

1. Connect the phone to the PC by USB, allow file transfer, and drag the
   `Mercenaries2Alpha` folder onto the phone's `Internal storage`.
2. On the phone, open it in My Files and tap the APK to install it (allow
   installs from unknown sources when asked).
3. Open the app. The first time, it opens the "All files access" page:
   switch it on (the game reads its files from that folder), then open the
   app again.

The game also reads from the app's own folder,
`Internal storage\Android\data\com.bstoney.m2alpha\files`, which needs no
permission (install and open the app once before copying there).

`Internal storage\Download\m2-alpha.log` holds what the game reported (the
first place to look when something goes wrong). The game's folder holds
`m2-alpha-crash.log`, the settings file and saves. Arguments the PC build
takes on its command line (`--no-boot`, `--spawn x,z`, ...) can be written
into `m2-alpha-args.txt` there.

## Building the APK (on Windows)

One-time setup:

1. The Android SDK command-line tools, then with its `sdkmanager`:
   `platform-tools`, `platforms;android-34`, `build-tools;34.0.0` and an NDK
   (`ndk;27.2.12479018` was used). Set `ANDROID_HOME` to the SDK and
   `ANDROID_NDK_ROOT` to the NDK.
2. A JDK (for `keytool`), on `PATH` or as `JAVA_HOME`.
3. `cargo install cargo-apk`.

Then run `android\build-apk.ps1`. It copies winit 0.30.13 from the cargo
registry into `android\target\winit` and applies
`winit-0.30.13-input.patch` (upstream winit reports an Android mouse and a
controller's sticks as touches), makes a signing key at
`%USERPROFILE%\.android\m2-alpha.keystore` if there is none, and builds
`android\target\release\apk\Mercenaries2Alpha.apk`.

`android\` is its own Cargo workspace so the desktop builds and CI never
compile it. Its library is `crates\m2-alpha\src\main.rs`; the Android entry
point is `crates\m2-alpha\src\android.rs`. When m2-alpha gains a
dependency, add it to `android\Cargo.toml` too.

## Movies (the menu's backdrop and music)

The game's movies are Bink files, played through a small FFmpeg the APK
carries as `lib/arm64-v8a/libffmpeg.so` (Bink in, raw pictures and sound
out; LGPL parts only). Build it once from an FFmpeg release tarball, from
Git Bash, before `build-apk.ps1`:

    sh android/build-ffmpeg.sh <ffmpeg-7.1.1 source folder>

Without it the APK still builds (with a warning) and skips movies.
