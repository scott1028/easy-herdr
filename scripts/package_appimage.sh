#!/usr/bin/env bash
# Wrap a static Linux easy-herdr binary in an AppImage.
# Usage: APPIMAGETOOL=... APPIMAGE_RUNTIME=... package_appimage.sh <easy-herdr-binary> <x86_64|aarch64> <output>
set -euo pipefail

ROOT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

if [[ $# -ne 3 ]]; then
  echo "usage: APPIMAGETOOL=... APPIMAGE_RUNTIME=... $0 <easy-herdr-binary> <x86_64|aarch64> <output>" >&2
  exit 2
fi
BINARY=$1
ARCH=$2
OUTPUT=$3
: "${APPIMAGETOOL:?set APPIMAGETOOL to the appimagetool executable}"
: "${APPIMAGE_RUNTIME:?set APPIMAGE_RUNTIME to the type2 runtime for $ARCH}"

APPDIR="$(mktemp -d)/easy-herdr.AppDir"
mkdir -p "$APPDIR/usr/bin"
install -m 0755 "$BINARY" "$APPDIR/usr/bin/easy-herdr"
ln -s usr/bin/easy-herdr "$APPDIR/AppRun"
cp "$ROOT_DIR/assets/herdr.svg" "$APPDIR/herdr.svg"
ln -s herdr.svg "$APPDIR/.DirIcon"
cat > "$APPDIR/easy-herdr.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=easy-herdr
Comment=Terminal based agent runtime for coding agents
Exec=easy-herdr
Icon=herdr
Terminal=true
Categories=Development;
EOF

# Runners have no FUSE, so appimagetool runs extracted.
ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGETOOL" --no-appstream \
  --runtime-file "$APPIMAGE_RUNTIME" "$APPDIR" "$OUTPUT"
rm -rf "$(dirname -- "$APPDIR")"
