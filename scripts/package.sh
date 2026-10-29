#!/usr/bin/env bash
# Build the dist profile and assemble dist/MatRust-<version>-<os>-<arch>.tar.gz (Linux x64 or macOS arm64,
# unsigned), then smoke-test it.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
root=$PWD
cargo=${CARGO:-cargo}
command -v "$cargo" >/dev/null || cargo="$HOME/.cargo/bin/cargo"
export PATH="$(dirname "$cargo"):$PATH"

version=$("$cargo" metadata --no-deps --format-version 1 |
  python3 -c 'import json,sys; print([p for p in json.load(sys.stdin)["packages"] if p["name"]=="matrust"][0]["version"])')
triple=$(rustc -vV | sed -n 's/^host: //p')
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) platform=linux-x64 ;;
  Darwin-arm64) platform=macos-arm64 ;;
  Darwin-x86_64) platform=macos-x64 ;;
  *) echo "unsupported platform $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac
name="MatRust-$version-$platform"
target=${CARGO_TARGET_DIR:-$root/target}

"$cargo" build --profile dist --features desktop --bins
bindir="$target/dist"

rm -rf "dist/$name" "dist/$name.tar.gz" "dist/$name.tar.gz.sha256"
mkdir -p "dist/$name"
for b in matrust matrust-desktop matrust-pdf-worker; do
  cp "$bindir/$b" "dist/$name/$b"
done
cp LICENSE "dist/$name/LICENSE"
mkdir -p "dist/$name/notices"
cp notices/*.txt THIRD_PARTY_LAPACK_LICENSE.txt "dist/$name/notices/"
cp scripts/QUICKSTART.txt "dist/$name/QUICKSTART.txt"
# The built-in `matrust bench` cases, a template for --cases (BENCHMARKS.md).
mkdir -p "dist/$name/bench"
cp bench/cases.csv "dist/$name/bench/"
python3 tool/third-party-licenses.py "dist/$name/THIRD-PARTY-LICENSES.txt" "$triple" "$root"

(cd dist && tar -czf "$name.tar.gz" "$name" && { command -v sha256sum >/dev/null && sha256sum "$name.tar.gz" || shasum -a 256 "$name.tar.gz"; } >"$name.tar.gz.sha256")
ls -l "dist/$name.tar.gz"

# Smoke test: unpack into a fresh directory and run from another fresh directory.
unpack=$(mktemp -d -t mr-unpack.XXXXXX) work=$(mktemp -d -t mr-work.XXXXXX)
trap 'rm -rf "$unpack" "$work"' EXIT
tar -xzf "dist/$name.tar.gz" -C "$unpack"
pkg="$unpack/$name"
cd "$work"
out=$("$pkg/matrust" --eval "x = 1:3; disp(sum(x))")
echo "matrust --eval: $out"
# disp currently prints 6.000000 for this input; accept any rendering of the number 6.
[ "$(echo "$out" | awk 'NF==1 && $1+0==6 && $1 ~ /^6(\.0*)?$/ {print "6"}')" = "6" ] || { echo "SMOKE FAIL: expected 6" >&2; exit 1; }
ver=$("$pkg/matrust-desktop" --version)
echo "matrust-desktop --version: $ver"
case "$ver" in *"$version"*) ;; *) echo "SMOKE FAIL: version $version not printed" >&2; exit 1 ;; esac
"$pkg/matrust" bench --only sum_1e6 --threads 1 --out "$work/bench.json" >/dev/null || { echo "SMOKE FAIL: matrust bench" >&2; exit 1; }
[ -s "$work/bench.json" ] || { echo "SMOKE FAIL: matrust bench wrote no result file" >&2; exit 1; }
echo "matrust bench: ok"
echo "SMOKE OK"
