# Fast local release: no installer bundle, release-fast profile, then launch.
# For a distributable setup.exe use:  .\build-installer.ps1
$ErrorActionPreference = "Stop"
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))
Set-Location $repoRoot

$exe = Join-Path $repoRoot "src-tauri\target\release-fast\window-hub.exe"

# rust-lld (llvm-tools) — speeds the final link that often sits at N-2/N.
$lldBin = Join-Path (rustc --print sysroot) "lib\rustlib\x86_64-pc-windows-msvc\bin"
if (Test-Path (Join-Path $lldBin "rust-lld.exe")) {
  $env:Path = "$lldBin;$env:Path"
} else {
  Write-Host "Tip: install faster linker with: rustup component add llvm-tools"
}

Write-Host "=== Stopping running Window Hub (if any) ==="
$remaining = @()
Get-Process -Name "window-hub" -ErrorAction SilentlyContinue | ForEach-Object {
  try {
    Stop-Process -Id $_.Id -Force -ErrorAction Stop
    Write-Host "Stopped PID $($_.Id)"
  } catch {
    Write-Host "Could not stop PID $($_.Id): Access denied (may be service/elevated)."
    $remaining += $_.Id
  }
}
Start-Sleep -Seconds 1
$still = @(Get-Process -Name "window-hub" -ErrorAction SilentlyContinue)
if ($still.Count -gt 0 -or $remaining.Count -gt 0) {
  Write-Host ""
  Write-Host "ERROR: window-hub.exe is still running. End it in Task Manager (or run this script as Administrator), then retry."
  Write-Host "Otherwise the linker cannot replace the binary and you may launch a stale build (asset not found: index.html)."
  exit 1
}

Write-Host "=== Window Hub: release-fast (no bundle, rust-lld) ==="
Write-Host "Note: first build of a profile recompiles everything; later edits mainly re-link."
# Touch frontend entry so tauri-build re-embeds dist into the binary.
if (Test-Path "dist\index.html") {
  (Get-Item "dist\index.html").LastWriteTime = Get-Date
}
npm run tauri:build:fast
if ($LASTEXITCODE -ne 0) {
  Write-Host ""
  Write-Host "Build failed."
  Write-Host "If you still see access denied on window-hub.exe, close it in Task Manager and retry."
  exit 1
}

if (-not (Test-Path -LiteralPath $exe)) {
  Write-Host ""
  Write-Host "Release exe not found:"
  Write-Host "  $exe"
  exit 1
}

Write-Host ""
Write-Host "=== Starting release-fast (tray on by default) ==="
Write-Host "  $exe"
Write-Host "  Emergency off: `$env:WH_DISABLE_TRAY='1'` before launch"
Start-Process -FilePath $exe
exit 0
