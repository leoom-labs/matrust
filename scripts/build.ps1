param([switch]$Desktop,[switch]$Check)
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
Push-Location (Join-Path $PSScriptRoot '..')
try {
    if ($Check) {
        & $cargo fmt --all -- --check
        if ($LASTEXITCODE -ne 0) {throw 'Formatting check failed'}
        if ($Desktop) {
            & $cargo build --release --features desktop --bin matrust-pdf-worker
            if ($LASTEXITCODE -ne 0) {throw 'PDF worker build failed'}
        }
        # Check release cfgs for every target without linking unused examples.
        if ($Desktop) {& $cargo check --release --all-targets --features desktop} else {& $cargo check --release --all-targets}
        if ($LASTEXITCODE -ne 0) {throw 'Release target check failed'}
        if ($Desktop) {& $cargo test --release --no-fail-fast --lib --bins --tests --features desktop} else {& $cargo test --release --no-fail-fast --lib --bins --tests}
        if ($LASTEXITCODE -ne 0) {throw 'Tests failed'}
        # Explicit target selection omits doctests; retain them as a separate gate.
        if ($Desktop) {& $cargo test --release --doc --features desktop} else {& $cargo test --release --doc}
        if ($LASTEXITCODE -ne 0) {throw 'Documentation tests failed'}
        & $cargo clippy --all-targets --features desktop -- -D warnings
        if ($LASTEXITCODE -ne 0) {throw 'Clippy failed'}
    }
    if ($Desktop) {& $cargo build --release --features desktop} else {& $cargo build --release}
    if ($LASTEXITCODE -ne 0) {throw 'Build failed'}
} finally {Pop-Location}
