# Run m2-alpha headless (offscreen --screenshot) for one comparison case, frames to <Out>\m2\.
# Waits, like polite.ps1, until no m2-alpha*/xenia* GPU program has been seen for 3 checks.
#   m2-capture.ps1 -Exe <m2-alpha.exe> -Install '<install dir>' -Out <case dir> -GameArgs @('--gait','sprint',...)
param(
  [Parameter(Mandatory)][string]$Exe,
  [Parameter(Mandatory)][string]$Install,
  [Parameter(Mandatory)][string]$Out,
  [string[]]$GameArgs = @(),
  [string]$Size = '1280x720',
  [int]$WallSeconds = 1800,
  [switch]$NoWait
)
if (-not $NoWait) {
  $ok = 0
  while ($ok -lt 3) {
    if (@(Get-Process 'm2-alpha*','xenia*' -ErrorAction SilentlyContinue).Count -lt 1) { $ok++ } else { $ok = 0 }
    if ($ok -lt 3) { Start-Sleep (Get-Random -Minimum 10 -Maximum 20) }
  }
}
$d = Join-Path $Out 'm2'
New-Item -ItemType Directory -Force $d | Out-Null
$env:WGPU_BACKEND = 'dx12'
$a = @('--install', "`"$Install`"", '--size', $Size, '--screenshot', "`"$d\m2.ppm`"") + $GameArgs
"m2-alpha $($a -join ' ')" | Set-Content "$Out\m2-command.txt"
$p = Start-Process -PassThru -WindowStyle Hidden -WorkingDirectory $d -FilePath $Exe -ArgumentList $a `
  -RedirectStandardOutput "$Out\m2.log" -RedirectStandardError "$Out\m2.err"
if (-not $p.WaitForExit($WallSeconds * 1000)) { Stop-Process -Id $p.Id -Force; "m2-alpha killed after $WallSeconds s" }
"m2-alpha exit $($p.ExitCode); frames: $((Get-ChildItem $d -Filter *.ppm).Count)"

