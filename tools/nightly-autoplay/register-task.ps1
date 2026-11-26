<#
.SYNOPSIS
  Registers (or with -Unregister removes) the "M2 nightly autoplay" Windows
  scheduled task for the signed-in user: daily at -Start, the window closing
  at -StopAt. Needs no admin: the task runs as this user, only while the user
  is signed in (Interactive logon), at normal (limited) rights.

.DESCRIPTION
  nightly.ps1 is copied to -TaskHome first, so the task doesn't depend on a
  worktree that may be removed. See docs/nightly-autoplay.md.

.EXAMPLE
  .\register-task.ps1                     # 01:00-07:00, issues as a dry run
  .\register-task.ps1 -PostIssues         # ...and post the issues
  .\register-task.ps1 -Unregister
#>
[CmdletBinding()]
param(
    [string]$Start = '01:00',
    [string]$StopAt = '07:00',
    [switch]$PostIssues,
    [switch]$Unregister,
    [string]$TaskName = 'M2 nightly autoplay',
    [string]$TaskHome = 'C:\Claude\scratch\nightly'
)
$ErrorActionPreference = 'Stop'

if ($Unregister) {
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
    Write-Output "removed the task '$TaskName'"
    return
}

New-Item -ItemType Directory -Force $TaskHome | Out-Null
$script = Join-Path $TaskHome 'nightly.ps1'
Copy-Item (Join-Path $PSScriptRoot 'nightly.ps1') $script -Force

$shell = (Get-Command pwsh -ErrorAction SilentlyContinue).Source
if (-not $shell) { $shell = (Get-Command powershell).Source }
$taskArgs = "-NoProfile -ExecutionPolicy Bypass -File `"$script`" -StopAt $StopAt"
if ($PostIssues) { $taskArgs += ' -PostIssues' }

$from = [datetime]::ParseExact($Start, 'HH:mm', $null)
$to = [datetime]::ParseExact($StopAt, 'HH:mm', $null)
if ($to -le $from) { $to = $to.AddDays(1) }
# The runner stops itself at -StopAt; the task's own limit is a backstop.
$limit = ($to - $from) + (New-TimeSpan -Minutes 20)

$action = New-ScheduledTaskAction -Execute $shell -Argument $taskArgs -WorkingDirectory $TaskHome
$trigger = New-ScheduledTaskTrigger -Daily -At $from
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit $limit -MultipleInstances IgnoreNew `
    -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
$principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel Limited
Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Settings $settings `
    -Principal $principal -Force `
    -Description "Mercenaries 2 Rust port: nightly autoplay of every contract, $Start-$StopAt (tools/nightly-autoplay)." |
    Out-Null
Write-Output "registered '$TaskName': daily $Start, stops $StopAt, runs $shell $taskArgs"
