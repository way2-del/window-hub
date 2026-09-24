# Compatibility entrypoint. Implementation lives in scripts/build.
& "$PSScriptRoot/scripts/build/build-and-run.ps1" @args
exit $LASTEXITCODE
