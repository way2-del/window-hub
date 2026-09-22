# Launch Window Hub with tray in **safe spy-only** mode.
# Does NOT enable explorer hook or registry soft-seed (those were hanging the process).
#
# Usage:
#   .\run-with-tray.ps1
#
# Opt-in riskier paths (not recommended until stable):
#   $env:WH_TRAY_HOOK="1"        # inject into explorer
#   $env:WH_TRAY_SOFT_SEED="1"   # registry IconSnapShot stubs
#   .\run-with-tray.ps1

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$env:WH_ENABLE_TRAY = "1"
# Explicitly leave hook/soft-seed off unless already set by caller.
if (-not $env:WH_TRAY_HOOK) { $env:WH_TRAY_HOOK = "0" }
if (-not $env:WH_TRAY_SOFT_SEED) { $env:WH_TRAY_SOFT_SEED = "0" }

Write-Host "WH_ENABLE_TRAY=$($env:WH_ENABLE_TRAY) WH_TRAY_HOOK=$($env:WH_TRAY_HOOK) WH_TRAY_SOFT_SEED=$($env:WH_TRAY_SOFT_SEED)"
& "$PSScriptRoot\build-and-run.ps1"
