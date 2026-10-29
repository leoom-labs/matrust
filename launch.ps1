$ErrorActionPreference='Stop'
Push-Location $PSScriptRoot
try {
    $application=Join-Path $PSScriptRoot 'target/release/matrust-desktop.exe'
    if (-not (Test-Path -LiteralPath $application)) {& ./scripts/build.ps1 -Desktop}
    & $application
} finally {Pop-Location}
