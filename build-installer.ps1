# Compatibility entrypoint. Implementation lives in scripts/build.
& "$PSScriptRoot/scripts/build/build-installer.ps1" @args
exit $LASTEXITCODE
