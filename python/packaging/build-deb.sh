#!/usr/bin/env bash
# Build an installable .deb package for Debian/Ubuntu.
# Usage:   bash packaging/build-deb.sh        (on a Debian/Ubuntu system)
# Result:  dist/backuptool_<version>_all.deb
set -euo pipefail

VERSION="1.2.4"
PKG="backuptool"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUILD="$ROOT/dist/deb"
OUT="$ROOT/dist"

rm -rf "$BUILD"; mkdir -p "$BUILD" "$OUT"

# --- file layout ---
install -d "$BUILD/usr/lib/backuptool/backuptool"
cp "$ROOT"/backuptool/*.py "$BUILD/usr/lib/backuptool/backuptool/"
cp -r "$ROOT"/backuptool/lang "$BUILD/usr/lib/backuptool/backuptool/"

install -d "$BUILD/usr/bin"
cat > "$BUILD/usr/bin/backuptool" <<'EOF'
#!/bin/sh
export PYTHONPATH=/usr/lib/backuptool
exec python3 -m backuptool "$@"
EOF
chmod 755 "$BUILD/usr/bin/backuptool"

# Desktop entry (GUI in the application menu)
install -d "$BUILD/usr/share/applications"
cp "$ROOT/packaging/backuptool.desktop" "$BUILD/usr/share/applications/"
cp "$ROOT/packaging/backuptool-admin.desktop" "$BUILD/usr/share/applications/"

# Admin launcher: runs the GUI with root rights but keeps the desktop session,
# so file dialogs work (plain `sudo backuptool-gui` breaks them).
install -m 755 "$ROOT/../scripts/backuptool-gui-admin" "$BUILD/usr/bin/backuptool-gui-admin"

# Application icon (hicolor theme, referenced by Icon=backuptool in the .desktop)
for size in 16 32 48 64 128 256 512; do
  src="$ROOT/packaging/icons/hicolor/${size}x${size}/apps/backuptool.png"
  [ -f "$src" ] || continue
  install -d "$BUILD/usr/share/icons/hicolor/${size}x${size}/apps"
  install -m 644 "$src" "$BUILD/usr/share/icons/hicolor/${size}x${size}/apps/backuptool.png"
done

# --- control file ---
install -d "$BUILD/DEBIAN"
cat > "$BUILD/DEBIAN/control" <<EOF
Package: $PKG
Version: $VERSION
Section: utils
Priority: optional
Architecture: all
Depends: python3 (>= 3.8)
Recommends: python3-pyside6.qtwidgets | python3-pyside6, rsync
Maintainer: Houshang Pezeshkpour (Atie) <houshang@pezeshkpour.eu>
Description: Cross-platform parallel incremental backup tool
 Backs up user data in parallel (multicore) and incrementally (mtime/size or
 SHA-256). Multiple backup sets for multiple systems, Qt6 GUI, restore including
 permissions/owner (also from exFAT targets via the manifest).
EOF

dpkg-deb --build --root-owner-group "$BUILD" "$OUT/${PKG}_${VERSION}_all.deb"
echo "Done: $OUT/${PKG}_${VERSION}_all.deb"
echo "Install:  sudo apt install ./dist/${PKG}_${VERSION}_all.deb"
echo "If python3-pyside6 is missing:  pip3 install PySide6"
