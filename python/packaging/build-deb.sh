#!/usr/bin/env bash
# Build an installable .deb package for Debian/Ubuntu (noarch / pure Python).
# Usage:   bash packaging/build-deb.sh
# Result:  dist/backuptool_<version>_all.deb
set -euo pipefail

VERSION="1.5.0"
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

# Convenience launcher for the GUI.
cat > "$BUILD/usr/bin/backuptool-gui" <<'EOF'
#!/bin/sh
export PYTHONPATH=/usr/lib/backuptool
exec python3 -m backuptool gui "$@"
EOF
chmod 755 "$BUILD/usr/bin/backuptool-gui"

# Desktop entry (GUI in the application menu)
install -d "$BUILD/usr/share/applications"
cp "$ROOT/packaging/backuptool.desktop" "$BUILD/usr/share/applications/"

# --- control file ---
install -d "$BUILD/DEBIAN"
cat > "$BUILD/DEBIAN/control" <<EOF
Package: $PKG
Version: $VERSION
Section: utils
Priority: optional
Architecture: all
Depends: python3 (>= 3.8)
Recommends: python3-pyside6.qtwidgets | python3-pyside6, python3-cryptography, python3-argon2, rsync
Maintainer: Houshang Pezeshkpour (Atie) <houshang@pezeshkpour.eu>
Description: Cross-platform parallel incremental backup tool
 Backs up user data in parallel (multicore) and incrementally (mtime/size or
 SHA-256). Multiple backup sets for multiple systems, Qt6 GUI, restore including
 permissions/owner (also from exFAT targets via the manifest). Selectable
 encryption (AES-256-GCM / ChaCha20-Poly1305) needs python3-cryptography +
 python3-argon2; the GUI needs PySide6.
EOF

dpkg-deb --build --root-owner-group "$BUILD" "$OUT/${PKG}_${VERSION}_all.deb"
echo "Done: $OUT/${PKG}_${VERSION}_all.deb"
echo "Install:  sudo apt install ./dist/${PKG}_${VERSION}_all.deb"
echo "GUI needs PySide6:        sudo apt install python3-pyside6  (or: pip3 install PySide6)"
echo "Encryption needs:         sudo apt install python3-cryptography python3-argon2"
