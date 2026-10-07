# Mercenaries 2 alpha on Android

The desktop game (`crates/m2-alpha`) built as an Android app, for a phone or
tablet with a Bluetooth keyboard and mouse (Samsung DeX works well). It
needs Vulkan (any phone from the last several years) and an arm64 CPU.

The game's files are never in the APK: the app reads your own copy from its
folder on the phone.

## What differs from the PC

- Movies (boot logos and cut-scenes) are skipped: they are decoded by
  `ffmpeg`, which the phone doesn't have.
- Sending the app to the background closes it (the renderer can't move to
  the new window Android gives it on return yet).
- The mouse: click in the game to capture it, as on the PC. Escape lets it
  go. While captured the cursor is hidden and the mouse turns the camera.

Everything else (the mission, HUD, saves, settings) is the PC build's.

## Copying the game to the phone

1. Install the APK and open it once. It creates its folder and closes or
   shows only the ground, since the game isn't there yet.
2. Connect the phone to the PC with a USB cable and allow file transfer.
3. In Explorer, open the phone's
   `Internal storage\Android\data\com.bstoney.m2alpha\files` folder.
4. Copy the `data` folder from your install
   (`C:\Program Files\EA Games\Mercenaries 2 World in Flames\data`) into it.
   The `Movies` folder inside it can be left out.
5. Open the app again.

Uninstalling the app deletes that folder (and the saves in it).

The same folder holds `m2-alpha.log` (what the game reported, the first
place to look when something goes wrong), `m2-alpha-crash.log`, the
settings file and saves. Arguments the PC build takes on its command line
(`--no-boot`, `--spawn x,z`, ...) can be written into `m2-alpha-args.txt`
there.

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
`winit-0.30.13-mouse.patch` (upstream winit reports an Android mouse as
touches), makes a signing key at `%USERPROFILE%\.android\m2-alpha.keystore`
if there is none, and builds
`android\target\release\apk\Mercenaries2Alpha.apk`.

`android\` is its own Cargo workspace so the desktop builds and CI never
compile it. Its library is `crates\m2-alpha\src\main.rs`; the Android entry
point is `crates\m2-alpha\src\android.rs`. When m2-alpha gains a
dependency, add it to `android\Cargo.toml` too.
