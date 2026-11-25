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
        # Tests use the gate profile (release without LTO, Cargo.toml); the PDF worker must sit in the same folder.
        if ($Desktop) {
            & $cargo build --profile gate --features desktop --bin matrust-pdf-worker
            if ($LASTEXITCODE -ne 0) {throw 'PDF worker build failed'}
        }
        # Check release cfgs for every target without linking unused examples.
        if ($Desktop) {& $cargo check --release --all-targets --features desktop} else {& $cargo check --release --all-targets}
        if ($LASTEXITCODE -ne 0) {throw 'Release target check failed'}
        if ($Desktop) {
            & $cargo test --profile gate --no-fail-fast --lib --bin matrust --bin matrust-desktop --test '*' --features desktop
            if ($LASTEXITCODE -ne 0) {throw 'Tests failed'}
            # --test '*', not --tests: --tests also runs the PDF worker's unit tests here, outside the guard below (hung 2026-11-02).
            # The PDF worker's test exe can print its result and then never exit (2026-10-30, twice): run it
            # alone and stop it once an ok result has stood for a minute.
            $pdfLog=Join-Path ([IO.Path]::GetTempPath()) 'matrust-pdf-worker-tests'
            $run=Start-Process -FilePath $cargo -ArgumentList 'test','--profile','gate','--features','desktop','--bin','matrust-pdf-worker' -NoNewWindow -PassThru -RedirectStandardOutput "$pdfLog.out" -RedirectStandardError "$pdfLog.err"
            $null=$run.Handle
            $deadline=(Get-Date).AddMinutes(30); $okSince=$null
            while (-not $run.HasExited -and (Get-Date) -lt $deadline) {
                Start-Sleep -Seconds 5
                $text=Get-Content -Raw -LiteralPath "$pdfLog.out" -ErrorAction SilentlyContinue
                if ($text -match 'test result: ok\.') {
                    if (-not $okSince) {$okSince=Get-Date} elseif (((Get-Date)-$okSince).TotalSeconds -ge 60) {break}
                }
            }
            Get-Content -LiteralPath "$pdfLog.err","$pdfLog.out" -ErrorAction SilentlyContinue
            if (-not $run.HasExited) {
                & taskkill /f /t /pid $run.Id | Out-Null
                if (-not $okSince) {throw 'PDF worker tests timed out'}
                'PDF worker tests passed but the test process did not exit; stopped it'
            } elseif ($run.ExitCode -ne 0) {throw 'PDF worker tests failed'}
        } else {
            & $cargo test --profile gate --no-fail-fast --lib --bins --tests
            if ($LASTEXITCODE -ne 0) {throw 'Tests failed'}
        }
        # Explicit target selection omits doctests; retain them as a separate gate.
        if ($Desktop) {& $cargo test --profile gate --doc --features desktop} else {& $cargo test --profile gate --doc}
        if ($LASTEXITCODE -ne 0) {throw 'Documentation tests failed'}
        & $cargo clippy --all-targets --features desktop -- -D warnings
        if ($LASTEXITCODE -ne 0) {throw 'Clippy failed'}
    }
    if ($Desktop) {& $cargo build --release --features desktop} else {& $cargo build --release}
    if ($LASTEXITCODE -ne 0) {throw 'Build failed'}
} finally {Pop-Location}
