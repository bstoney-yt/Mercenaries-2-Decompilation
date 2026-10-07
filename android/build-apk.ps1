# Builds the Android APK (arm64) of the alpha: android\target\release\apk\Mercenaries2Alpha.apk.
#
# Needs (see README.md): ANDROID_HOME (an SDK with a platform and build-tools),
# ANDROID_NDK_ROOT (an NDK), a JDK's keytool on PATH or under JAVA_HOME,
# and cargo-apk (`cargo install cargo-apk`).
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$root = Split-Path $here -Parent

foreach ($v in 'ANDROID_HOME', 'ANDROID_NDK_ROOT') {
    if (-not [Environment]::GetEnvironmentVariable($v)) { throw "$v is not set (see android\README.md)" }
}
rustup target add aarch64-linux-android | Out-Null

# winit with the mouse patch, copied from the registry (the root workspace
# pins the same 0.30.13).
$winit = Join-Path $here 'target\winit'
if (-not (Test-Path (Join-Path $winit 'Cargo.toml'))) {
    Push-Location $root; cargo fetch; Pop-Location
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
    $src = Get-ChildItem (Join-Path $cargoHome 'registry\src') -Directory |
        ForEach-Object { Join-Path $_.FullName 'winit-0.30.13' } |
        Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $src) { throw 'winit-0.30.13 is not in the cargo registry' }
    New-Item -ItemType Directory -Force (Split-Path $winit) | Out-Null
    Copy-Item -Recurse $src $winit
    Push-Location $winit
    git apply -p1 (Join-Path $here 'winit-0.30.13-mouse.patch')
    Pop-Location
    if (-not (Select-String -Quiet -Path (Join-Path $winit 'src\platform_impl\android\mod.rs') -Pattern 'MouseRelative')) {
        Remove-Item -Recurse -Force $winit
        throw 'the winit mouse patch did not apply'
    }
}

# A signing key of this PC's own, kept outside the repo and the build
# folder: the phone only installs an update signed with the same key.
$keystore = Join-Path $env:USERPROFILE '.android\m2-alpha.keystore'
if (-not (Test-Path $keystore)) {
    New-Item -ItemType Directory -Force (Split-Path $keystore) | Out-Null
    $keytool = if ($env:JAVA_HOME) { Join-Path $env:JAVA_HOME 'bin\keytool.exe' } else { 'keytool' }
    & $keytool -genkeypair -keystore $keystore -storepass android -keypass android -alias m2alpha `
        -keyalg RSA -keysize 2048 -validity 10000 -dname 'CN=Mercenaries 2 Alpha' | Out-Null
}
$env:CARGO_APK_RELEASE_KEYSTORE = $keystore
$env:CARGO_APK_RELEASE_KEYSTORE_PASSWORD = 'android'

Push-Location $here
try {
    cargo apk build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo apk build failed ($LASTEXITCODE)" }
} finally { Pop-Location }
Get-Item (Join-Path $here 'target\release\apk\Mercenaries2Alpha.apk')
