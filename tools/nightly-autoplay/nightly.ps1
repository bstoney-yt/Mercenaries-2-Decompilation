<#
.SYNOPSIS
  The nightly autoplay run on the dev VM: build origin/main (unless -Exe is
  given), play each contract by autoplay until the window closes, then open
  or update one GitHub issue per break (a dry run unless -PostIssues).

.DESCRIPTION
  See docs/nightly-autoplay.md. Logs, pictures and results go under -Out
  (on the NAS); the exes run from a local disk (the share won't start one).
  The GPU lock (C:\Claude\scratch\gpu.lock) and the m2-alpha* slot rule are
  handled by m2-nightly itself.

.EXAMPLE
  # One contract, now, with an exe already built (a trial run):
  .\nightly.ps1 -Exe C:\path\m2-alpha.exe -Nightly C:\path\m2-nightly.exe -Only PirCon001 -MaxMinutes 50

.EXAMPLE
  # The full night (what the scheduled task runs):
  .\nightly.ps1 -StopAt 07:00
#>
[CmdletBinding()]
param(
    [string]$Install = 'C:\Games\Mercenaries 2 World in Flames',
    [string]$Out = 'Z:\mercs2\scratch\nightly',
    # Local time the window closes: the run then playing is stopped and the GPU released.
    [string]$StopAt = '07:00',
    # Minutes from now to stop instead (0: use -StopAt).
    [int]$MaxMinutes = 0,
    # An m2-alpha.exe to use; without it origin/main is built.
    [string]$Exe,
    # The m2-nightly.exe to use with -Exe (default: beside -Exe).
    [string]$Nightly,
    # The commit -Exe was built from, for the report and issues.
    [string]$Head = 'unknown',
    [string[]]$Only,
    # More m2-nightly run options, as is (see m2-nightly --help).
    [string[]]$RunnerArgs,
    [switch]$Jobs,
    [switch]$PostIssues,
    [string]$Repo = 'bstoney-yt/Mercenaries-2-Decompilation',
    [int]$KnownBugs = 698,
    [string]$MainCheckout = 'C:\Claude\Mercenaries-2-Decompilation',
    [string]$BuildTree = 'C:\Claude\scratch\nightly-build',
    [string]$Target = 'C:\Claude\scratch\target-nightly',
    [string]$Bin = 'C:\Claude\scratch\nightly-bin'
)
$ErrorActionPreference = 'Stop'

function Get-Deadline([string]$StopAt, [int]$MaxMinutes) {
    if ($MaxMinutes -gt 0) {
        return [DateTimeOffset]::Now.AddMinutes($MaxMinutes).ToUnixTimeSeconds()
    }
    $stop = [datetime]::ParseExact($StopAt, 'HH:mm', $null)
    if ($stop -le (Get-Date)) { $stop = $stop.AddDays(1) }
    return ([DateTimeOffset]$stop).ToUnixTimeSeconds()
}

function Build-Main([string]$MainCheckout, [string]$BuildTree, [string]$Target, [string]$Bin) {
    # A detached worktree of the main checkout, kept at origin/main; the
    # main checkout's own branch is never touched.
    if (-not (Test-Path $BuildTree)) {
        git -C $MainCheckout fetch origin main
        git -C $MainCheckout worktree add --detach $BuildTree origin/main
    }
    git -C $BuildTree fetch origin main
    git -C $BuildTree checkout --detach FETCH_HEAD
    if ($LASTEXITCODE -ne 0) { throw "could not move $BuildTree to origin/main" }
    $sha = (git -C $BuildTree rev-parse HEAD).Trim()
    $env:CARGO_TARGET_DIR = $Target
    cargo build --release --manifest-path "$BuildTree\Cargo.toml" -p m2-alpha -p m2-nightly
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    $dir = Join-Path $Bin $sha
    New-Item -ItemType Directory -Force $dir | Out-Null
    Copy-Item "$Target\release\m2-alpha.exe", "$Target\release\m2-nightly.exe" $dir -Force
    # Keep the three newest builds.
    Get-ChildItem $Bin -Directory | Sort-Object LastWriteTime -Descending |
        Select-Object -Skip 3 | Remove-Item -Recurse -Force
    return @{ Exe = "$dir\m2-alpha.exe"; Nightly = "$dir\m2-nightly.exe"; Head = $sha }
}

New-Item -ItemType Directory -Force $Out | Out-Null
$stamp = Get-Date -Format 'yyyyMMdd-HHmm'
$runnerLog = Join-Path $Out "runner-$stamp.log"
Start-Transcript -Path $runnerLog -Append | Out-Null
try {
    $deadline = Get-Deadline $StopAt $MaxMinutes
    if ($Exe) {
        if (-not $Nightly) { $Nightly = Join-Path (Split-Path $Exe) 'm2-nightly.exe' }
        if (-not (Test-Path $Nightly)) { throw "no m2-nightly.exe at $Nightly (pass -Nightly)" }
        $build = @{ Exe = $Exe; Nightly = $Nightly; Head = $Head }
    } else {
        $build = Build-Main $MainCheckout $BuildTree $Target $Bin
    }
    $runArgs = @('run', '--exe', $build.Exe, '--install', $Install, '--out', $Out,
        '--deadline-unix', "$deadline", '--head', $build.Head)
    if ($Only) { $runArgs += @('--only', (($Only | ForEach-Object { $_ -split ',' }) -join ',')) }
    if ($Jobs) { $runArgs += '--jobs' }
    if ($RunnerArgs) { $runArgs += $RunnerArgs }
    Write-Output "m2-nightly $($runArgs -join ' ')"
    $lines = & $build.Nightly @runArgs
    $lines | Write-Output
    $night = $lines | Where-Object { $_ -match 'night\.json$' } | Select-Object -Last 1
    if (-not $night) { throw 'the runner wrote no night.json' }
    $issueArgs = @('issues', '--night', $night, '--repo', $Repo, '--known-bugs', "$KnownBugs")
    if (-not $PostIssues) { $issueArgs += '--dry-run' }
    $issues = & $build.Nightly @issueArgs
    $issues | Out-File (Join-Path (Split-Path $night) 'issues.txt') -Encoding utf8
    $issues | Write-Output
} finally {
    Stop-Transcript | Out-Null
}
