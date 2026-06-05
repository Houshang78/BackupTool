#!/usr/bin/env bash
# Build for macOS:  (1) a launchable backuptool.app  and  (2) a .pkg installer.
# Usage:   bash packaging/build-macos.sh        (on macOS)
# Result:  dist/backuptool.app  and  dist/backuptool-<version>.pkg
set -euo pipefail

VERSION="1.2.4"
IDENT="eu.pezeshkpour.backuptool"   # bundle identifier (Atie / Houshang Pezeshkpour)
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/dist"
APP="$OUT/backuptool.app"
PAYLOAD="$OUT/pkgroot"

rm -rf "$APP" "$PAYLOAD"; mkdir -p "$OUT"

# ----- (1) .app-Bundle -----
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/backuptool"
cp "$ROOT"/backuptool/*.py "$APP/Contents/Resources/backuptool/"
cp -r "$ROOT"/backuptool/lang "$APP/Contents/Resources/backuptool/"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>backuptool</string>
  <key>CFBundleDisplayName</key><string>backuptool</string>
  <key>CFBundleIdentifier</key><string>$IDENT</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleExecutable</key><string>backuptool</string>
  <key>CFBundleIconFile</key><string>backuptool</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
</dict></plist>
EOF

# Application icon
if [ -f "$ROOT/packaging/backuptool.icns" ]; then
  cp "$ROOT/packaging/backuptool.icns" "$APP/Contents/Resources/backuptool.icns"
fi

cat > "$APP/Contents/MacOS/backuptool" <<'EOF'
#!/bin/bash
DIR="$(cd "$(dirname "$0")/../Resources" && pwd)"
export PYTHONPATH="$DIR"
PY="$(command -v python3 || echo /usr/bin/python3)"
exec "$PY" -m backuptool
EOF
chmod 755 "$APP/Contents/MacOS/backuptool"
echo "App built: $APP   (double-click launches the GUI; needs 'pip3 install PySide6')"

# ----- (2) .pkg installer (CLI into /usr/local) -----
mkdir -p "$PAYLOAD/usr/local/lib/backuptool/backuptool" "$PAYLOAD/usr/local/bin"
cp "$ROOT"/backuptool/*.py "$PAYLOAD/usr/local/lib/backuptool/backuptool/"
cp -r "$ROOT"/backuptool/lang "$PAYLOAD/usr/local/lib/backuptool/backuptool/"
cat > "$PAYLOAD/usr/local/bin/backuptool" <<'EOF'
#!/bin/sh
export PYTHONPATH=/usr/local/lib/backuptool
PY="$(command -v python3 || echo /usr/bin/python3)"
exec "$PY" -m backuptool "$@"
EOF
chmod 755 "$PAYLOAD/usr/local/bin/backuptool"

if command -v pkgbuild >/dev/null; then
  pkgbuild --root "$PAYLOAD" --identifier "$IDENT" --version "$VERSION" \
           --install-location / "$OUT/backuptool-$VERSION.pkg"
  echo "Package built: $OUT/backuptool-$VERSION.pkg"
  echo "Install:  sudo installer -pkg dist/backuptool-$VERSION.pkg -target /"
else
  echo "pkgbuild not found (macOS only) – skipping .pkg."
fi
echo "Note: for the GUI run once  pip3 install PySide6"
