# Compatibility entrypoint. Implementation lives in scripts/build.
& "$PSScriptRoot/scripts/build/run-with-tray.ps1" @args
exit $LASTEXITCODE
