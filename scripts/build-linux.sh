#!/usr/bin/env bash
# Build ALL Linux artifacts in one go: Rust CLI+GUI binaries and the Python .deb.
# Run on a Debian/Ubuntu system:  bash scripts/build-linux.sh
# Output: dist/linux/
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/dist/linux"
mkdir -p "$OUT"

echo "==> Rust (CLI + GUI, release)"
cd "$ROOT/rust"
cargo build --release --bin backuptool
if ! cargo build --release --features gui --bin backuptool-gui; then
  echo "!! GUI build failed. Install Slint deps:" >&2
  echo "   sudo apt install -y libfontconfig1-dev libxcb1-dev libxkbcommon-dev libwayland-dev" >&2
fi
cp -f target/release/backuptool "$OUT/" 2>/dev/null || true
cp -f target/release/backuptool-gui "$OUT/" 2>/dev/null || true

echo "==> Python (.deb)"
cd "$ROOT/python"
if command -v dpkg-deb >/dev/null; then
  bash packaging/build-deb.sh
  cp -f dist/*.deb "$OUT/" 2>/dev/null || true
else
  echo "!! dpkg-deb not found – skipping .deb (install: sudo apt install dpkg-dev)" >&2
fi

echo
echo "Artifacts in $OUT:"
ls -lh "$OUT"
