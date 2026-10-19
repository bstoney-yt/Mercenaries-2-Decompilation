# Builds the Android APK (arm64) of the alpha: android\target\release\apk\Mercenaries2Alpha.apk.
#
# Needs (see README.md): ANDROID_HOME (an SDK with a platform and build-tools),
# ANDROID_NDK_ROOT (an NDK), a JDK's keytool on PATH or under JAVA_HOME,
# and cargo-apk (`cargo install cargo-apk`).
$ErrorActionPreference = 'Stop'
# Native tools report progress on stderr, which 'Stop' would take for a
# failure when the output is redirected: run them with 'Continue' and check
# their exit codes.
function Invoke-Tool([scriptblock] $run) {
    $ErrorActionPreference = 'Continue'
    & $run 2>&1 | ForEach-Object { "$_" }
}
$here = $PSScriptRoot
$root = Split-Path $here -Parent

foreach ($v in 'ANDROID_HOME', 'ANDROID_NDK_ROOT') {
    if (-not [Environment]::GetEnvironmentVariable($v)) { throw "$v is not set (see android\README.md)" }
}
Invoke-Tool { rustup target add aarch64-linux-android } | Out-Null

# winit with the input patch, copied from the registry (the root workspace
# pins the same 0.30.13). A copy made from an older patch is redone.
$winit = Join-Path $here 'target\winit'
$patched = Join-Path $winit 'src\platform_impl\android\mod.rs'
if ((Test-Path $patched) -and -not (Select-String -Quiet -Path $patched -Pattern 'is_joystick_class')) {
    Remove-Item -Recurse -Force $winit
}
if (-not (Test-Path (Join-Path $winit 'Cargo.toml'))) {
    Push-Location $root; Invoke-Tool { cargo fetch } | Out-Null; Pop-Location
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
    $src = Get-ChildItem (Join-Path $cargoHome 'registry\src') -Directory |
        ForEach-Object { Join-Path $_.FullName 'winit-0.30.13' } |
        Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $src) { throw 'winit-0.30.13 is not in the cargo registry' }
    New-Item -ItemType Directory -Force (Split-Path $winit) | Out-Null
    Copy-Item -Recurse $src $winit
    # The registry source has LF endings; a checkout with core.autocrlf
    # gives the patch CRLF ones, which git apply then rejects.
    $patch = Join-Path (Split-Path $winit) 'winit-input.lf.patch'
    [IO.File]::WriteAllText($patch, ([IO.File]::ReadAllText((Join-Path $here 'winit-0.30.13-input.patch')) -replace "`r`n", "`n"))
    Push-Location $winit
    git apply -p1 $patch
    Pop-Location
    if (-not (Select-String -Quiet -Path $patched -Pattern 'is_joystick_class')) {
        Remove-Item -Recurse -Force $winit
        throw 'the winit input patch did not apply'
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
    Invoke-Tool { cargo apk build --release }
    if ($LASTEXITCODE -ne 0) { throw "cargo apk build failed ($LASTEXITCODE)" }
} finally { Pop-Location }
Get-Item (Join-Path $here 'target\release\apk\Mercenaries2Alpha.apk')
