<#
.SYNOPSIS
  Stage 0 desktop benchmark: process start -> UI ready, idle memory and idle CPU
  of the whole process tree (launcher + WebView2 children).
.EXAMPLE
  pwsh scripts/bench-startup.ps1 -Exe target/release/night-client.exe -Runs 5
#>
param(
  [string]$Exe = "target/release/night-client.exe",
  [int]$Runs = 5,
  [int]$IdleSeconds = 10
)
$ErrorActionPreference = "Stop"
$exePath = (Resolve-Path $Exe).Path

function Get-Tree([int]$rootId) {
  $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name
  $ids = [System.Collections.Generic.List[int]]::new(); $ids.Add($rootId)
  for ($i = 0; $i -lt $ids.Count; $i++) {
    $all | Where-Object { $_.ParentProcessId -eq $ids[$i] } | ForEach-Object { $ids.Add([int]$_.ProcessId) }
  }
  $ids | ForEach-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue }
}

$results = @()
for ($r = 1; $r -le $Runs; $r++) {
  # Release builds log only to the rotating file in %LOCALAPPDATA%\NightClient\logs.
  $logDir = Join-Path $env:LOCALAPPDATA "NightClient\logs"
  $find = { @(Get-ChildItem $logDir -Filter "night-client*.log" -ErrorAction SilentlyContinue | Select-String -Pattern "ui_ready_ms=(\d+)") }
  $before = (& $find).Count
  $p = Start-Process -FilePath $exePath -PassThru
  $ready = $null
  $deadline = (Get-Date).AddSeconds(30)
  while (-not $ready -and (Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 100
    $m = & $find
    if ($m.Count -gt $before) { $ready = [int]$m[-1].Matches[0].Groups[1].Value }
  }
  Start-Sleep -Seconds $IdleSeconds
  $tree = Get-Tree $p.Id
  $cpu0 = ($tree | ForEach-Object { $_.TotalProcessorTime.TotalMilliseconds } | Measure-Object -Sum).Sum
  Start-Sleep -Seconds 5
  $tree = Get-Tree $p.Id
  $cpu1 = ($tree | ForEach-Object { $_.TotalProcessorTime.TotalMilliseconds } | Measure-Object -Sum).Sum
  $ws = ($tree | Measure-Object -Property WorkingSet64 -Sum).Sum / 1MB
  $priv = ($tree | Measure-Object -Property PrivateMemorySize64 -Sum).Sum / 1MB
  $cores = [Environment]::ProcessorCount
  $results += [pscustomobject]@{
    Run = $r
    UiReadyMs = $ready
    Processes = $tree.Count
    WorkingSetMB = [math]::Round($ws, 1)
    PrivateMB = [math]::Round($priv, 1)
    IdleCpuPct = [math]::Round(($cpu1 - $cpu0) / 5000 / $cores * 100, 2)
  }
  $tree | Sort-Object Id -Descending | ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
  Start-Sleep -Seconds 2
}
$results | Format-Table -AutoSize | Out-String -Width 200
