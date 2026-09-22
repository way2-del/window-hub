# Monitor window-hub CPU/RAM/handles while reproducing hangs.
# Only tracks window-hub.exe + its child msedgewebview2 (not system-wide Edge).
# Usage: powershell -File docs/monitor-resources.ps1
# Log: %TEMP%\window-hub-resource.log

$out = Join-Path $env:TEMP "window-hub-resource.log"
"" | Set-Content $out
Write-Host "Logging to $out (Ctrl+C to stop). Watching window-hub + child WebViews only."

function Get-ChildPids([int]$parentId) {
  try {
    Get-CimInstance Win32_Process -Filter "ParentProcessId=$parentId" -ErrorAction SilentlyContinue |
      ForEach-Object { [int]$_.ProcessId }
  } catch { @() }
}

$prevCpu = @{}
while ($true) {
  $ts = Get-Date -Format "HH:mm:ss.fff"
  $hub = @(Get-Process -Name "window-hub" -ErrorAction SilentlyContinue)
  if ($hub.Count -eq 0) {
    Add-Content $out "$ts`tnone"
    Start-Sleep -Seconds 1
    continue
  }

  $watch = New-Object System.Collections.Generic.List[object]
  foreach ($h in $hub) {
    $watch.Add($h) | Out-Null
    $kids = Get-ChildPids $h.Id
    # one level of grandchildren common for WebView2 helper
    $all = [System.Collections.Generic.HashSet[int]]::new()
    foreach ($k in $kids) { [void]$all.Add($k); foreach ($g in (Get-ChildPids $k)) { [void]$all.Add($g) } }
    foreach ($id in $all) {
      $p = Get-Process -Id $id -ErrorAction SilentlyContinue
      if ($p -and $p.ProcessName -match 'msedgewebview2|window-hub') { $watch.Add($p) | Out-Null }
    }
  }

  $anyHung = $false
  foreach ($p in $watch) {
    $id = $p.Id
    $cpu = $p.CPU
    $delta = if ($prevCpu.ContainsKey($id)) { [math]::Round($cpu - $prevCpu[$id], 2) } else { 0 }
    $prevCpu[$id] = $cpu
    $ws = [math]::Round($p.WorkingSet64 / 1MB, 1)
    $resp = $p.Responding
    if (-not $resp) { $anyHung = $true }
    $line = "$ts`tpid=$id`tname=$($p.ProcessName)`tResponding=$resp`tCPU_delta=${delta}s`tWS=${ws}MB`thandles=$($p.HandleCount)`tthreads=$($p.Threads.Count)"
    Add-Content $out $line
    if (-not $resp) { Write-Host "HUNG $line" -ForegroundColor Red }
    elseif ($delta -ge 0.8) { Write-Host "HIGH_CPU $line" -ForegroundColor Yellow }
  }

  $db = Join-Path $env:APPDATA "window-hub\window-hub.db"
  $wal = Join-Path $env:APPDATA "window-hub\window-hub.db-wal"
  if (Test-Path $db) {
    $dbKb = [math]::Round((Get-Item $db).Length / 1KB)
    $walKb = if (Test-Path $wal) { [math]::Round((Get-Item $wal).Length / 1KB) } else { 0 }
    $lockNote = ""
    try {
      $fs = [IO.File]::Open($db, 'Open', 'ReadWrite', 'None')
      $fs.Close()
    } catch {
      $lockNote = " DB_LOCKED"
    }
    Add-Content $out "$ts`tdb=${dbKb}KB wal=${walKb}KB$lockNote"
    if ($walKb -gt 1024) { Write-Host "WAL_LARGE ${walKb}KB" -ForegroundColor Yellow }
  }

  if ($anyHung) { Write-Host "$ts process Not Responding" -ForegroundColor Red }
  Start-Sleep -Seconds 1
}
