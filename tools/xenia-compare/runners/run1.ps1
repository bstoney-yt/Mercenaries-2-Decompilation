param([string]$Name, [string]$Route)
$env:XENIA_GAME = 'Z:\mercs2\scratch\ghidra360-recovery\disc1\default.xex'
Set-Location C:\Claude\scratch\wt-xenia-compare\tools\xenia-compare
.\xenia-route.ps1 -NoWait -Audio -XeniaArgs '--headless=true' -Route "Z:\mercs2\scratch\xenia-compare\$Route-route.txt" -Out "Z:\mercs2\scratch\xenia-compare\$Name"
