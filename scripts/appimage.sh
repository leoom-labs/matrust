#!/usr/bin/env bash
# Wrap an unpacked Linux release folder into dist/<name>.AppImage (x86-64).
# Usage: scripts/appimage.sh dist/MatRust-<version>-linux-x64
# Downloads appimagetool 1.9.1 and the type2 runtime 20251108 once into ~/.cache/matrust-appimage,
# checked against pinned SHA-256 sums.
set -euo pipefail
src=${1:?unpacked release folder}
name=$(basename "$src")
cd "$(dirname "${BASH_SOURCE[0]}")/.."
cache=${XDG_CACHE_HOME:-$HOME/.cache}/matrust-appimage
mkdir -p "$cache"
fetch() { # url sha256 file
  if [ ! -f "$cache/$3" ]; then curl -fsSL -o "$cache/$3.part" "$1"; mv "$cache/$3.part" "$cache/$3"; fi
  echo "$2  $cache/$3" | sha256sum -c --quiet || { rm -f "$cache/$3"; echo "checksum mismatch: $3" >&2; exit 1; }
}
fetch https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage \
  ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0 appimagetool-1.9.1
fetch https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64 \
  2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d runtime-20251108
chmod +x "$cache/appimagetool-1.9.1"

app=$(mktemp -d)/MatRust.AppDir
mkdir -p "$app/usr/bin" "$app/usr/share/doc/matrust"
cp "$src"/matrust "$src"/matrust-desktop "$src"/matrust-pdf-worker "$app/usr/bin/"
cp -r "$src"/LICENSE "$src"/THIRD-PARTY-LICENSES.txt "$src"/notices "$src"/QUICKSTART.txt "$src"/bench "$app/usr/share/doc/matrust/"
cp assets/linux/matrust.desktop assets/linux/matrust.png "$app/"
ln -s usr/bin/matrust-desktop "$app/AppRun"
mkdir -p dist
out="dist/$name.AppImage"
# APPIMAGE_EXTRACT_AND_RUN: no FUSE needed to run the tool itself (CI runners)
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$cache/appimagetool-1.9.1" --no-appstream \
  --runtime-file "$cache/runtime-20251108" "$app" "$out"
rm -rf "$(dirname "$app")"
(cd dist && sha256sum "$name.AppImage" > "$name.AppImage.sha256")
# smoke: the AppImage starts and reports its version without FUSE
APPIMAGE_EXTRACT_AND_RUN=1 "$out" --version
ls -l "$out"
