# Free Rust/Tauri build cache under src-tauri/target (can grow to tens of GB).
# Prefer keeping only one warm profile (debug XOR release-fast) day-to-day.
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

Write-Host "=== Stopping Window Hub (if any) ==="
Get-Process -Name "window-hub" -ErrorAction SilentlyContinue | ForEach-Object {
  try {
    Stop-Process -Id $_.Id -Force -ErrorAction Stop
    Write-Host "Stopped PID $($_.Id)"
  } catch {
    Write-Host "Could not stop PID $($_.Id): $($_.Exception.Message)"
  }
}
Start-Sleep -Seconds 1

$target = Join-Path $PSScriptRoot "..\src-tauri\target"
if (-not (Test-Path $target)) {
  Write-Host "No target dir — nothing to clean."
  exit 0
}

$sizeBefore = (Get-ChildItem $target -Recurse -File -ErrorAction SilentlyContinue |
  Measure-Object Length -Sum).Sum
Write-Host ("Before: {0:N1} GB" -f ($sizeBefore / 1GB))

Push-Location (Join-Path $PSScriptRoot "..\src-tauri")
try {
  cargo clean
} finally {
  Pop-Location
}

$left = if (Test-Path $target) {
  (Get-ChildItem $target -Recurse -File -ErrorAction SilentlyContinue |
    Measure-Object Length -Sum).Sum
} else { 0 }
Write-Host ("After:  {0:N1} GB" -f ($left / 1GB))
Write-Host "Next build will be a cold compile."
exit 0
