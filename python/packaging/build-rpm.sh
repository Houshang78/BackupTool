#!/usr/bin/env bash
# Build an installable .rpm package for Fedora/RHEL/openSUSE (noarch / pure Python).
# Usage:   bash packaging/build-rpm.sh
# Result:  dist/backuptool-<version>-1.noarch.rpm
set -euo pipefail

VERSION="1.6.0"
PKG="backuptool"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TOP="$ROOT/dist/rpmbuild"
OUT="$ROOT/dist"
SPEC="$TOP/SPECS/${PKG}.spec"

rm -rf "$TOP"; mkdir -p "$TOP"/{SPECS,RPMS,BUILD,BUILDROOT} "$OUT"

cat > "$SPEC" <<SPEC
# Pure-Python noarch package; skip strip/bytecompile so it builds off-target too.
%global __os_install_post %{nil}
%global debug_package %{nil}

Name:           ${PKG}
Version:        ${VERSION}
Release:        1
Summary:        Cross-platform parallel incremental backup tool
License:        GPL-3.0-or-later
URL:            https://github.com/Houshang78/BackupTool
BuildArch:      noarch
Requires:       python3 >= 3.8
Recommends:     python3-pyside6
Recommends:     python3-cryptography
Recommends:     python3-argon2-cffi

%description
Backs up user data in parallel (multicore) and incrementally (mtime/size or
SHA-256). Multiple backup sets for multiple systems, a Qt6 GUI, restore
including permissions/owner (also from exFAT targets via the manifest).
Selectable encryption (AES-256-GCM / ChaCha20-Poly1305) needs
python3-cryptography + python3-argon2-cffi; the GUI needs PySide6.

%install
rm -rf %{buildroot}
install -d %{buildroot}/usr/lib/backuptool/backuptool
cp ${ROOT}/backuptool/*.py %{buildroot}/usr/lib/backuptool/backuptool/
cp -r ${ROOT}/backuptool/lang %{buildroot}/usr/lib/backuptool/backuptool/
install -d %{buildroot}/usr/bin
cat > %{buildroot}/usr/bin/backuptool <<'LAUNCH'
#!/bin/sh
export PYTHONPATH=/usr/lib/backuptool
exec python3 -m backuptool "\$@"
LAUNCH
chmod 755 %{buildroot}/usr/bin/backuptool
cat > %{buildroot}/usr/bin/backuptool-gui <<'LAUNCH'
#!/bin/sh
export PYTHONPATH=/usr/lib/backuptool
exec python3 -m backuptool gui "\$@"
LAUNCH
chmod 755 %{buildroot}/usr/bin/backuptool-gui
install -d %{buildroot}/usr/share/applications
cp ${ROOT}/packaging/backuptool.desktop %{buildroot}/usr/share/applications/

%files
/usr/lib/backuptool
/usr/bin/backuptool
/usr/bin/backuptool-gui
/usr/share/applications/backuptool.desktop

%changelog
* Sun Jun 14 2026 Houshang Pezeshkpour <houshang@pezeshkpour.eu> - ${VERSION}-1
- Decommission/clone, selectable encryption, SSD/HDD-aware secure wipe.
SPEC

rpmbuild --define "_topdir $TOP" -bb "$SPEC"
RPM="$(find "$TOP/RPMS" -name '*.rpm' | head -1)"
cp "$RPM" "$OUT/"
echo "Done: $OUT/$(basename "$RPM")"
echo "Install (Fedora):   sudo dnf install ./dist/$(basename "$RPM")"
echo "GUI needs PySide6:  sudo dnf install python3-pyside6   (or: pip3 install PySide6)"
echo "Encryption needs:   sudo dnf install python3-cryptography python3-argon2-cffi"
