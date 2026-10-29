#!/usr/bin/env bash
# Build MatRust for the browser into web/pkg: matrust_web.js (ES module,
# wasm-bindgen --target web), matrust_web_bg.wasm and worker.js.
# Needs the wasm32-unknown-unknown Rust target; installs the matching
# wasm-bindgen-cli when it is missing. Runs wasm-opt only if it is on PATH.
set -euo pipefail
web="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$HOME/.cargo/bin:$PATH"

version="$(sed -n 's/^wasm-bindgen = "=\([0-9.]*\)"$/\1/p' "$web/Cargo.toml")"
[ -n "$version" ] || { echo "build.sh: no pinned wasm-bindgen version in web/Cargo.toml" >&2; exit 1; }

cargo build --release --target wasm32-unknown-unknown --manifest-path "$web/Cargo.toml"
target="${CARGO_TARGET_DIR:-$web/target}"
wasm="$target/wasm32-unknown-unknown/release/matrust_web.wasm"

# The CLI must equal the crate version, or the generated glue does not match.
if [ "$(wasm-bindgen --version 2>/dev/null | cut -d' ' -f2)" != "$version" ]; then
    cargo install wasm-bindgen-cli --version "$version" --locked
fi

rm -rf "$web/pkg"
wasm-bindgen --target web --out-dir "$web/pkg" "$wasm"
cp "$web/worker.js" "$web/pkg/worker.js"

out="$web/pkg/matrust_web_bg.wasm"
if command -v wasm-opt >/dev/null; then
    # Rust's default wasm features; a failed pass keeps the unoptimized module.
    if wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
        --enable-mutable-globals --enable-reference-types --enable-multivalue \
        "$out" -o "$out.opt"; then
        mv "$out.opt" "$out"
    else
        rm -f "$out.opt"
        echo "build.sh: wasm-opt failed; keeping the unoptimized module" >&2
    fi
else
    echo "build.sh: wasm-opt not on PATH; skipped"
fi

raw=$(wc -c < "$out")
gz=$(gzip -9 -c "$out" | wc -c)
awk -v raw="$raw" -v gz="$gz" \
    'BEGIN { printf "matrust_web_bg.wasm: %.2f MB raw, %.2f MB gzipped\n", raw / 1048576, gz / 1048576 }'
