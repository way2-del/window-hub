# Fast local release: no installer bundle, release-fast profile, then launch.
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$exe = Join-Path $PSScriptRoot "src-tauri\target\release-fast\window-hub.exe"

# rust-lld (llvm-tools) — speeds the final link that often sits at N-2/N.
$lldBin = Join-Path (rustc --print sysroot) "lib\rustlib\x86_64-pc-windows-msvc\bin"
if (Test-Path (Join-Path $lldBin "rust-lld.exe")) {
  $env:Path = "$lldBin;$env:Path"
} else {
  Write-Host "Tip: install faster linker with: rustup component add llvm-tools"
}

Write-Host "=== Stopping running Window Hub (if any) ==="
Get-Process -Name "window-hub" -ErrorAction SilentlyContinue | ForEach-Object {
  try {
    Stop-Process -Id $_.Id -Force -ErrorAction Stop
  } catch {
    Write-Host "Could not stop PID $($_.Id): Access denied (may be service/elevated)."
    Write-Host "End it in Task Manager, or re-run this script as Administrator."
  }
}
Start-Sleep -Seconds 1

Write-Host "=== Window Hub: release-fast (no bundle, rust-lld) ==="
Write-Host "Note: first build of a profile recompiles everything; later edits mainly re-link."
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
Write-Host "=== Starting release-fast ==="
Write-Host "  $exe"
Start-Process -FilePath $exe
exit 0
