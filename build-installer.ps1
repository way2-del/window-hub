# Build NSIS installer (setup .exe). Not the same as build-and-run.ps1
# (that script is local-only and uses --no-bundle).
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

Write-Host "=== Stopping running Window Hub (if any) ==="
Get-Process -Name "window-hub" -ErrorAction SilentlyContinue | ForEach-Object {
  try {
    Stop-Process -Id $_.Id -Force -ErrorAction Stop
    Write-Host "Stopped PID $($_.Id)"
  } catch {
    Write-Host "Could not stop PID $($_.Id): $($_.Exception.Message)"
  }
}
Start-Sleep -Seconds 1

Write-Host "=== Window Hub: release + NSIS installer ==="
npm run tauri:build:bundle
if ($LASTEXITCODE -ne 0) {
  Write-Host ""
  Write-Host "Installer build failed."
  exit 1
}

$bundleDir = Join-Path $PSScriptRoot "src-tauri\target\release\bundle\nsis"
$setup = Get-ChildItem -Path $bundleDir -Filter "*.exe" -ErrorAction SilentlyContinue |
  Sort-Object LastWriteTime -Descending |
  Select-Object -First 1

Write-Host ""
if ($setup) {
  Write-Host "Installer ready:"
  Write-Host "  $($setup.FullName)"
} else {
  Write-Host "Build finished but no NSIS .exe found under:"
  Write-Host "  $bundleDir"
  exit 1
}
exit 0
