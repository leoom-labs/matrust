param([switch]$Mat73Research,[switch]$VerifyOnly)
$ErrorActionPreference='Stop'
$toolsDir=Join-Path $PSScriptRoot '../tools'
if (!$VerifyOnly) {New-Item -ItemType Directory -Force -Path $toolsDir | Out-Null}
$compilers=@(
    @{name='llvm-mingw-20261006-ucrt-x86_64';sha256='317492c456aa27ee607a5919f1d2d38dcdc1112516a24d0bf4b00d078f52d17a'},
    @{name='llvm-mingw-20261006-msvcrt-x86_64';sha256='24847b37a951c8583c3f7ec061de2eb87f6394215c8ef3fdc4565ed0835396e1'}
)
foreach ($tool in $compilers) {
    $compiler=Join-Path $toolsDir ($tool.name+'/bin/x86_64-w64-mingw32-clang.exe')
    if (!(Test-Path -LiteralPath $compiler)) {
        if ($VerifyOnly) {throw "Missing local compiler: $($tool.name)"}
        $archive=Join-Path $toolsDir ($tool.name+'.zip')
        if (!(Test-Path -LiteralPath $archive) -or (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $tool.sha256) {
            $download='https://github.com/mstorsjo/llvm-mingw/releases/download/20261006/'+$tool.name+'.zip'
            & curl.exe -L --fail --silent --show-error $download -o $archive
            if ($LASTEXITCODE -ne 0) {throw "LLVM-MinGW download failed: $($tool.name)"}
        }
        if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $tool.sha256) {
            throw "LLVM-MinGW archive checksum mismatch: $($tool.name)"
        }
        Expand-Archive -LiteralPath $archive -DestinationPath $toolsDir -Force
    }
    $compilerVersion=& $compiler --version
    if ($LASTEXITCODE -ne 0) {throw "Compiler verification failed: $($tool.name)"}
    Write-Output $compilerVersion[0]
}
$hdfVenv=Join-Path $toolsDir 'v73-python'
$hdfPython=Join-Path $hdfVenv 'Scripts/python.exe'
if (!(Test-Path -LiteralPath $hdfPython)) {
    if ($VerifyOnly) {throw 'Missing local CMake/Ninja environment; run this installer without -VerifyOnly'}
    & python.exe -m venv $hdfVenv
    if ($LASTEXITCODE -ne 0) {throw 'Creating the build-tool environment failed; install Python 3 and retry'}
}
if (!$VerifyOnly) {
    & $hdfPython -m pip install 'cmake==4.4.4' 'ninja==1.13.2'
    if ($LASTEXITCODE -ne 0) {throw 'Installing CMake/Ninja failed'}
    if ($Mat73Research) {
        & $hdfPython -m pip install 'h5py==3.16.0' 'hdf5storage==0.2.2' 'numpy==2.5.4'
        if ($LASTEXITCODE -ne 0) {throw 'Installing optional MAT research tools failed'}
    }
}
@'
import importlib.metadata as m
assert m.version("cmake") == "4.4.4"
assert m.version("ninja") == "1.13.2"
print("Pinned CMake and Ninja verified")
'@ | & $hdfPython -
if ($LASTEXITCODE -ne 0) {throw 'CMake/Ninja verification failed'}
if ($Mat73Research) {
    @'
import importlib.metadata as m
for name, version in {"h5py":"3.16.0", "hdf5storage":"0.2.2", "numpy":"2.5.4"}.items():
    assert m.version(name) == version, name
print("Optional MAT research tools verified")
'@ | & $hdfPython -
    if ($LASTEXITCODE -ne 0) {throw 'Research-tool verification failed'}
}
Write-Output 'Local build tools verified. Python and these tools are not application runtime dependencies. Run scripts/build.ps1 -Desktop -Check.'
