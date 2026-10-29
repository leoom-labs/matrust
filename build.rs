fn main() {
    println!("cargo:rerun-if-env-changed=MATRUST_WINDOWS_LIB_DIR");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu")
    {
        let directory = std::env::var_os("MATRUST_WINDOWS_LIB_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
                    .join("tools/llvm-mingw-20261006-ucrt-x86_64/x86_64-w64-mingw32/lib")
            });
        let import_library = directory.join("libshlwapi.a");
        if import_library.is_file() {
            // Import only the missing Windows API library. Adding LLVM-MinGW's
            // whole library path would shadow Rust's matching C runtime.
            let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
            std::fs::copy(&import_library, output.join("libshlwapi.a"))
                .expect("copy Windows shell API import library");
            println!("cargo:rerun-if-changed={}", import_library.display());
            println!("cargo:rustc-link-search=native={}", output.display());
        }
        // Recent MinGW headers call __timezone; Rust's bundled older MSVCRT
        // archive exposes only the underlying data. Link the compiler-supplied
        // accessor alone so Rust retains its matching startup/runtime objects.
        let toolchain = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
            .join("tools/llvm-mingw-20261006-msvcrt-x86_64");
        let ar = toolchain.join("bin/llvm-ar.exe");
        if ar.is_file() {
            let runtime = toolchain.join("x86_64-w64-mingw32/lib/libmsvcrt.a");
            println!("cargo:rerun-if-changed={}", runtime.display());
            let extracted = std::process::Command::new(&ar)
                .arg("p")
                .arg(&runtime)
                .arg("lib64_libmsvcrt_extra_a-__timezone.o")
                .output()
                .expect("extract compiler timezone accessor");
            assert!(extracted.status.success() && !extracted.stdout.is_empty());
            let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
            let object = output.join("mingw-timezone.o");
            std::fs::write(&object, extracted.stdout).expect("write timezone accessor");
            assert!(
                std::process::Command::new(ar)
                    .arg("rcs")
                    .arg(output.join("libmatrust-timezone.a"))
                    .arg(object)
                    .status()
                    .expect("archive timezone accessor")
                    .success()
            );
            println!("cargo:rustc-link-search=native={}", output.display());
            println!("cargo:rustc-link-lib=static=matrust-timezone");
        }
    }
}
