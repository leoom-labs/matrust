$ErrorActionPreference = 'Stop'
$toolsDir = Join-Path $PSScriptRoot '../tools'
New-Item -ItemType Directory -Force $toolsDir | Out-Null
$artifacts = @(
    @{ Name='ghidra.zip'; Url='https://github.com/NationalSecurityAgency/ghidra/releases/download/Ghidra_12.1.4_build/ghidra_12.1.4_PUBLIC_20260921.zip'; Hash='' },
    @{ Name='jdk.zip'; Url='https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.12.1%2B1/OpenJDK21U-jdk_x64_windows_hotspot_21.0.12.1_1.zip'; Hash='f9d6e191ab098c0d416e7d588a24420a8621cd2f4720dab2459b8b7b2d2d8b4e' }
)
foreach ($artifact in $artifacts) {
    $destination = Join-Path $toolsDir $artifact.Name
    & curl.exe -L --fail --silent --show-error $artifact.Url -o $destination
    if ($LASTEXITCODE -ne 0) { throw "Download failed: $($artifact.Name)" }
    if ($artifact.Hash -and (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash -ne $artifact.Hash) { throw 'Checksum mismatch' }
    Write-Output "Downloaded $($artifact.Name); extracting"
    Expand-Archive -LiteralPath $destination -DestinationPath $toolsDir -Force
}
Write-Output 'Local Ghidra and JDK ready in tools/'
