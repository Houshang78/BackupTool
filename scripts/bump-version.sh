#!/usr/bin/env bash
# Bump the project version everywhere (both implementations + packaging + docs).
# Usage:  bash scripts/bump-version.sh 1.2.1
#
# Replaces the exact current version string project-wide (except Cargo.lock and the
# CHANGELOG history), refreshes Cargo.lock, and prepends a CHANGELOG stub.
set -euo pipefail

NEW="${1:-}"
[ -n "$NEW" ] || { echo "Usage: $0 <new-version>   e.g. 1.2.1" >&2; exit 1; }
echo "$NEW" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' \
    || { echo "Version must be MAJOR.MINOR.PATCH" >&2; exit 1; }

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

CUR="$(grep -m1 '^version' python/pyproject.toml | sed -E 's/.*"([^"]+)".*/\1/')"
[ -n "$CUR" ] || { echo "Could not detect current version from pyproject.toml" >&2; exit 1; }
if [ "$CUR" = "$NEW" ]; then
  echo "Already at $NEW — nothing to do."
  exit 0
fi
echo "Bumping $CUR -> $NEW"

# Files containing the exact current version (skip Cargo.lock, CHANGELOG, .git).
esc_cur="${CUR//./\\.}"
files="$(grep -rl --exclude=Cargo.lock --exclude=CHANGELOG.md --exclude-dir=.git -F "$CUR" . || true)"
for f in $files; do
  sed -i.bak "s/${esc_cur}/${NEW}/g" "$f" && rm -f "$f.bak"
  echo "  updated $f"
done

# Refresh Cargo.lock's own package entry (reads the new Cargo.toml version).
if command -v cargo >/dev/null; then
  (cd rust && cargo check >/dev/null 2>&1 || true)
  echo "  refreshed rust/Cargo.lock"
fi

# Prepend a CHANGELOG stub (edit the bullet points before committing).
if [ -f CHANGELOG.md ]; then
  tmp="$(mktemp)"
  { head -3 CHANGELOG.md; printf '\n## [%s]\n### Added\n- TODO: describe changes\n' "$NEW"; tail -n +4 CHANGELOG.md; } > "$tmp"
  mv "$tmp" CHANGELOG.md
  echo "  added CHANGELOG stub for $NEW"
fi

echo
echo "Done. Review with 'git diff', edit the CHANGELOG, then:"
echo "  git commit -am \"Bump version to $NEW\" && git tag -a v$NEW -m \"backuptool v$NEW\" && git push --follow-tags"
