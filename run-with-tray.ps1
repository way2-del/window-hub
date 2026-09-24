# Launch Window Hub (tray is always on — same as build-and-run.ps1).
# Kept as an alias so old habits / docs still work.
#
# Emergency off for debugging:
#   $env:WH_DISABLE_TRAY="1"; .\build-and-run.ps1
#
# Opt-in riskier paths:
#   $env:WH_TRAY_HOOK="1"
#   $env:WH_TRAY_SOFT_SEED="1"

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

Write-Host "Tray is on by default (spy-safe). Use WH_DISABLE_TRAY=1 only to debug without tray."
if ($env:WH_TRAY_HOOK -or $env:WH_TRAY_SOFT_SEED) {
  Write-Host "WH_TRAY_HOOK=$($env:WH_TRAY_HOOK) WH_TRAY_SOFT_SEED=$($env:WH_TRAY_SOFT_SEED)"
}
& "$PSScriptRoot\build-and-run.ps1"
