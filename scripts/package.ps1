# Build the dist profile and assemble dist\MatRust-<version>-windows-x64.zip, then smoke-test it.
$ErrorActionPreference='Stop'
if (Test-Path Env:HDF5_DIR) {
    throw 'Verified MatRust builds require bundled patched static HDF5; remove HDF5_DIR from the build environment and retry.'
}
$cargo=Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe'
$rustc=Join-Path $env:USERPROFILE '.cargo/bin/rustc.exe'
$sysroot=& $rustc --print sysroot
$rustBin=Join-Path $sysroot 'lib/rustlib/x86_64-pc-windows-gnu/bin'
# Rust's matching 64-bit dlltool must precede unrelated MinGW tools.
$llvmBin=Join-Path $PSScriptRoot '../tools/llvm-mingw-20261006-ucrt-x86_64/bin'
$env:PATH="$llvmBin;$(Join-Path $rustBin 'self-contained');$rustBin;$(Split-Path $cargo);$env:PATH"
$hdfCompiler=Join-Path $PSScriptRoot '../tools/llvm-mingw-20261006-msvcrt-x86_64/bin'
$hdfBuildTools=Join-Path $PSScriptRoot '../tools/v73-python/Scripts'
if (Test-Path (Join-Path $hdfCompiler 'x86_64-w64-mingw32-clang.exe')) {
    $hdfCompiler=(Resolve-Path -LiteralPath $hdfCompiler).Path
    $hdfBuildTools=(Resolve-Path -LiteralPath $hdfBuildTools).Path
    $env:CC=Join-Path $hdfCompiler 'x86_64-w64-mingw32-clang.exe'
    $env:CXX=Join-Path $hdfCompiler 'x86_64-w64-mingw32-clang++.exe'
    $env:AR=Join-Path $hdfCompiler 'llvm-ar.exe'
    $env:PATH="$hdfBuildTools;$hdfCompiler;$env:PATH"
    $env:CMAKE_GENERATOR='Ninja'
}
$root=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $root
try {
    # cargo writes progress to stderr; do not let that stop the script.
    $ErrorActionPreference='Continue'
    $meta=(& $cargo metadata --no-deps --format-version 1) -join "`n"
    if ($LASTEXITCODE -ne 0) {throw 'cargo metadata failed'}
    $version=(($meta | ConvertFrom-Json).packages | Where-Object {$_.name -eq 'matrust'}).version
    $triple='x86_64-pc-windows-gnu'
    $name="MatRust-$version-windows-x64"
    $target=if ($env:CARGO_TARGET_DIR) {$env:CARGO_TARGET_DIR} else {Join-Path $root 'target'}

    & $cargo build --profile dist --features desktop --bins
    if ($LASTEXITCODE -ne 0) {throw 'Build failed'}
    $ErrorActionPreference='Stop'

    $pkg=Join-Path $root "dist/$name"
    $zip=Join-Path $root "dist/$name.zip"
    if (Test-Path $pkg) {Remove-Item -Recurse -Force $pkg}
    if (Test-Path $zip) {Remove-Item -Force $zip}
    New-Item -ItemType Directory -Force $pkg | Out-Null
    foreach ($b in 'matrust','matrust-desktop','matrust-pdf-worker') {
        Copy-Item (Join-Path $target "dist/$b.exe") $pkg
    }
    Copy-Item LICENSE $pkg
    New-Item -ItemType Directory -Force (Join-Path $pkg 'notices') | Out-Null
    Copy-Item notices/*.txt,THIRD_PARTY_LAPACK_LICENSE.txt (Join-Path $pkg 'notices')
    Copy-Item scripts/QUICKSTART.txt $pkg
    # The built-in `matrust bench` cases, a template for --cases (BENCHMARKS.md).
    New-Item -ItemType Directory -Force (Join-Path $pkg 'bench') | Out-Null
    Copy-Item bench/cases.csv (Join-Path $pkg 'bench')
    $py=Get-Command python -ErrorAction SilentlyContinue
    if (-not $py) {$py=Get-Command python3 -ErrorAction SilentlyContinue}
    if (-not $py) {throw 'python not found (needed for tool/third-party-licenses.py)'}
    $ErrorActionPreference='Continue'
    & $py.Source tool/third-party-licenses.py (Join-Path $pkg 'THIRD-PARTY-LICENSES.txt') $triple $root
    if ($LASTEXITCODE -ne 0) {throw 'Licence generation failed'}
    $ErrorActionPreference='Stop'

    # Windows tar (bsdtar) writes zip entries with forward slashes; PowerShell 5
    # Compress-Archive writes backslashes, which unzip on Linux/macOS mangles.
    & "$env:SystemRoot\System32\tar.exe" -a -c -f $zip -C (Split-Path $pkg) $name
    if ($LASTEXITCODE -ne 0) {throw 'zip creation failed'}
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive=[IO.Compression.ZipFile]::OpenRead($zip)
    try {
        $bad=@($archive.Entries | Where-Object {$_.FullName.Contains('\')})
        if ($bad.Count -gt 0) {throw "zip has backslash paths: $($bad[0].FullName)"}
    } finally {$archive.Dispose()}
    $hash=(Get-FileHash -Algorithm SHA256 $zip).Hash.ToLower()
    [IO.File]::WriteAllText("$zip.sha256","$hash  $name.zip`n")
    Write-Output ("{0} {1} bytes" -f $zip,(Get-Item $zip).Length)

    # Smoke test: unpack into a fresh directory and run from another fresh directory.
    $unpack=Join-Path ([IO.Path]::GetTempPath()) ("matrust-unpack-"+[guid]::NewGuid())
    $work=Join-Path ([IO.Path]::GetTempPath()) ("matrust-work-"+[guid]::NewGuid())
    New-Item -ItemType Directory $unpack,$work | Out-Null
    try {
        Expand-Archive -Path $zip -DestinationPath $unpack
        $exe=Join-Path $unpack $name
        Push-Location $work
        try {
            $ErrorActionPreference='Continue'
            $out=((& (Join-Path $exe 'matrust.exe') --eval 'x = 1:3; disp(sum(x))') -join '').Trim()
            $rc=$LASTEXITCODE
            Write-Output "matrust --eval: $out"
            if ($rc -ne 0 -or $out -notmatch '^6(\.0*)?$') {throw "SMOKE FAIL: expected 6, got '$out'"}
            # Piped through Out-String: PowerShell waits for a GUI-subsystem program only in a pipeline.
            $ver=(& (Join-Path $exe 'matrust-desktop.exe') --version | Out-String).Trim()
            $rc=$LASTEXITCODE
            Write-Output "matrust-desktop --version: $ver"
            if ($rc -ne 0 -or -not $ver.Contains($version)) {throw "SMOKE FAIL: version $version not printed"}
            $benchOut=Join-Path $work 'bench.json'
            & (Join-Path $exe 'matrust.exe') bench --only sum_1e6 --threads 1 --out $benchOut | Out-Null
            $rc=$LASTEXITCODE
            if ($rc -ne 0 -or -not (Test-Path $benchOut)) {throw "SMOKE FAIL: matrust bench exited $rc"}
            Write-Output 'matrust bench: ok'
            $ErrorActionPreference='Stop'
        } finally {Pop-Location}
    } finally {
        Remove-Item -Recurse -Force $unpack,$work -ErrorAction SilentlyContinue
    }
    Write-Output 'SMOKE OK'
} finally {Pop-Location}
