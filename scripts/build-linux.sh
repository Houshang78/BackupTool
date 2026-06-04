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

# rustup installs cargo under ~/.cargo/bin, which sudo drops from PATH
# (secure_path). Locate it so the script also works under sudo.
if ! command -v cargo >/dev/null 2>&1; then
  for env_file in "$HOME/.cargo/env" "/root/.cargo/env" \
                  ${SUDO_USER:+"/home/$SUDO_USER/.cargo/env"}; do
    [ -f "$env_file" ] && . "$env_file" && break
  done
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "!! cargo not found." >&2
  echo "   Install Rust (https://rustup.rs):  curl https://sh.rustup.rs -sSf | sh" >&2
  echo "   Building needs no root — prefer:  bash scripts/build-linux.sh  (no sudo)." >&2
  echo "   If you must use sudo:  sudo env \"PATH=\$PATH\" bash scripts/build-linux.sh" >&2
  exit 1
fi

if ! cargo build --release --bin backuptool; then
  echo "!! cargo build failed." >&2
  echo "   If the error mentions 'lock file version 4', your Cargo is older than 1.78." >&2
  echo "   Fix:  rustup update stable     (or: rm rust/Cargo.lock && retry)" >&2
  exit 1
fi
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
