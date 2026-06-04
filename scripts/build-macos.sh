#!/usr/bin/env bash
# Build ALL macOS artifacts in one go: Rust CLI+GUI binaries, Python .app and .pkg.
# Run on macOS:  bash scripts/build-macos.sh
# Output: dist/macos/
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/dist/macos"
mkdir -p "$OUT"

echo "==> Rust (CLI + GUI, release)"
cd "$ROOT/rust"
cargo build --release --bin backuptool
cargo build --release --features gui --bin backuptool-gui
cp -f target/release/backuptool "$OUT/" 2>/dev/null || true
cp -f target/release/backuptool-gui "$OUT/" 2>/dev/null || true

echo "==> Python (.app + .pkg)"
cd "$ROOT/python"
bash packaging/build-macos.sh
cp -Rf dist/backuptool.app "$OUT/" 2>/dev/null || true
cp -f dist/*.pkg "$OUT/" 2>/dev/null || true

echo
echo "Artifacts in $OUT:"
ls -lh "$OUT"
echo "Note: the Python .app/.pkg need 'pip3 install PySide6' for the GUI at runtime."
