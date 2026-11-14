# m2-alpha side of the three comparison cases (A air drops, B rifle sprint, C RPG launch).
param([Parameter(Mandatory)][string]$Exe, [Parameter(Mandatory)][string]$Install, [string]$Root = 'Z:\mercs2\scratch\xenia-compare')
$cap = Join-Path $PSScriptRoot '..\m2-capture.ps1'
$a = (0..45 | % { 2 * $_ }) -join ','
& $cap -Exe $Exe -Install $Install -Out "$Root\A-airdrops" -WallSeconds 900 -GameArgs @('--hero','mattias','--mission','--autoplay','--mission-seconds','95','--mission-shots',$a)
& $cap -Exe $Exe -Install $Install -Out "$Root\B-sprint" -WallSeconds 600 -GameArgs @('--hero','mattias','--gait','sprint','--walk-seconds','5','--check-shots','1,2,3,4','--behind')
# --fire shoots 3 ticks (0.05 s) before the picture: the launch flash and back-blast.
& $cap -Exe $Exe -Install $Install -Out "$Root\C-rpg\side" -WallSeconds 600 -GameArgs @('--hero','mattias','--weapon','rpg','--aim','--fire')
& $cap -Exe $Exe -Install $Install -Out "$Root\C-rpg\behind" -WallSeconds 600 -GameArgs @('--hero','mattias','--weapon','rpg','--aim','--fire','--behind')

