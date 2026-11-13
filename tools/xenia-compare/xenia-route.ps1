# Wait until no m2-alpha*/xenia* GPU program has been seen for 3 checks, then run xc.py.
param([Parameter(Mandatory)][string]$Route, [Parameter(Mandatory)][string]$Out, [switch]$Audio, [switch]$NoWait, [string[]]$XeniaArgs = @())
$here = $PSScriptRoot
if (-not $NoWait) {
  $ok = 0
  while ($ok -lt 3) {
    if (@(Get-Process 'm2-alpha*','xenia*' -ErrorAction SilentlyContinue).Count -lt 1) { $ok++ } else { $ok = 0 }
    if ($ok -lt 3) { Start-Sleep (Get-Random -Minimum 10 -Maximum 20) }
  }
}
$game = if ($env:XENIA_GAME) { $env:XENIA_GAME } else { throw 'set XENIA_GAME to the extracted default.xex' }
$w = @('--with','pillow','--with','numpy','--with','winrt-Windows.Media.Ocr','--with','winrt-Windows.Graphics.Imaging','--with','winrt-Windows.Storage',
       '--with','winrt-Windows.Storage.Streams','--with','winrt-Windows.Foundation','--with','winrt-Windows.Foundation.Collections','--with','winrt-Windows.Globalization')
$a = @('run', $Route, '--out', $Out, '--game', $game)
if ($Audio) { $a += '--audio' }
foreach ($x in $XeniaArgs) { $a += "--extra=$x" }
uv run -q --no-project @w python -I "$here\xc.py" @a
