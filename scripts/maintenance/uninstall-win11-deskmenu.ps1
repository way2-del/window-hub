<#
.SYNOPSIS
  Remove leftover Window Hub Win11 desktop context-menu integration.

.DESCRIPTION
  Unregisters sparse package WindowHub.DesktopMenu (if present), deletes HKCU
  shell / ContextMenuHandlers / CLSID keys, clears Shell Extensions cache and
  AppModel PolicyCache, and removes desktop-menu.json.

  Run after removing the deskmenu feature from the app, or when the modern
  desktop menu still shows "Window Hub" / "创建收纳盒" / "桌面设置…".
#>
param(
  [switch]$RestartExplorer
)

$ErrorActionPreference = "Continue"
$PkgName = "WindowHub.DesktopMenu"
$Clsid = "{8F4E2A1B-3C5D-4E6F-9A0B-1C2D3E4F5A6B}"

function Write-Step([string]$Message) {
  Write-Host "[deskmenu-uninstall] $Message"
}

# ── AppX sparse package ───────────────────────────────────────────────
Get-AppxPackage -Name $PkgName -ErrorAction SilentlyContinue | ForEach-Object {
  Write-Step "Remove-AppxPackage $($_.PackageFullName)"
  Remove-AppxPackage -Package $_.PackageFullName -ErrorAction SilentlyContinue
}

try {
  $id = [Security.Principal.WindowsIdentity]::GetCurrent()
  $prin = New-Object Security.Principal.WindowsPrincipal($id)
  if ($prin.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Get-AppxProvisionedPackage -Online -ErrorAction SilentlyContinue |
      Where-Object { $_.DisplayName -eq $PkgName } |
      ForEach-Object {
        Write-Step "Remove-AppxProvisionedPackage $($_.PackageName)"
        Remove-AppxProvisionedPackage -Online -PackageName $_.PackageName -ErrorAction SilentlyContinue
      }
  }
} catch {}

# ── HKCU shell / COM ──────────────────────────────────────────────────
$hkcuPaths = @(
  "Software\Classes\Directory\Background\shell\ Window Hub",
  "Software\Classes\DesktopBackground\shell\ Window Hub",
  "Software\Classes\Directory\Background\shellex\ContextMenuHandlers\ Window Hub",
  "Software\Classes\DesktopBackground\shellex\ContextMenuHandlers\ Window Hub",
  "Software\Classes\CLSID\$Clsid"
)
foreach ($rel in $hkcuPaths) {
  $full = Join-Path "HKCU:" $rel
  if (Test-Path -LiteralPath $full) {
    Remove-Item -LiteralPath $full -Recurse -Force -ErrorAction SilentlyContinue
    Write-Step "Deleted $rel"
  }
}

foreach ($shellBase in @(
  "Software\Classes\Directory\Background\shell",
  "Software\Classes\DesktopBackground\shell"
)) {
  $basePath = Join-Path "HKCU:" $shellBase
  if (-not (Test-Path -LiteralPath $basePath)) { continue }
  Get-ChildItem -LiteralPath $basePath -ErrorAction SilentlyContinue | ForEach-Object {
    $name = $_.PSChildName
    $kill = ($name -match '^\d+_WH_') -or ($name -match 'Window Hub')
    $cmdPath = Join-Path $_.PSPath "command"
    if (-not $kill -and (Test-Path -LiteralPath $cmdPath)) {
      $val = (Get-ItemProperty -LiteralPath $cmdPath -ErrorAction SilentlyContinue)."(default)"
      if ($val -is [string] -and $val -match "--wh-desktop-action") { $kill = $true }
    }
    if ($kill) {
      Remove-Item -LiteralPath $_.PSPath -Recurse -Force -ErrorAction SilentlyContinue
      Write-Step "Deleted verb $shellBase\$name"
    }
  }
}

# ── Shell extension + AppModel caches ─────────────────────────────────
$cached = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Shell Extensions\Cached"
if (Test-Path $cached) {
  (Get-ItemProperty $cached).PSObject.Properties |
    Where-Object { $_.Name -like "*$Clsid*" } |
    ForEach-Object {
      Remove-ItemProperty -Path $cached -Name $_.Name -Force -ErrorAction SilentlyContinue
      Write-Step "Cleared Shell Extensions cache entry"
    }
}

$policy = "HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\PolicyCache"
if (Test-Path $policy) {
  Get-ChildItem $policy -ErrorAction SilentlyContinue |
    Where-Object { $_.PSChildName -match "WindowHub\.DesktopMenu" } |
    ForEach-Object {
      Remove-Item -LiteralPath $_.PSPath -Recurse -Force -ErrorAction SilentlyContinue
      Write-Step "Deleted PolicyCache $($_.PSChildName)"
    }
}

$cfg = Join-Path $env:LOCALAPPDATA "window-hub\desktop-menu.json"
if (Test-Path -LiteralPath $cfg) {
  Remove-Item -LiteralPath $cfg -Force
  Write-Step "Deleted desktop-menu.json"
}

if ($RestartExplorer) {
  Write-Step "Restarting Explorer…"
  Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 2
  if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) {
    Start-Process explorer
  }
}

Write-Step "Done. If items still appear, sign out / restart Explorer, then rebuild Window Hub from a tree without deskmenu."
