#!/usr/bin/env bash
# build, tag, and publish presshold to AUR.
#
# Usage: ./release.sh
#
# Prerequisites:
#   - Version already bumped in Cargo.toml and PKGBUILD
#   - All changes committed and pushed to GitHub
#   - aur-presshold/ is a clone of the AUR repo (in the same directory as this script)
#     (if missing, run: git clone ssh://aur@aur.archlinux.org/presshold.git aur-presshold)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AUR_DIR="$SCRIPT_DIR/aur-presshold"

# Read version from Cargo.toml
VERSION=$(grep '^version' "$SCRIPT_DIR/Cargo.toml" | head -1 | sed 's/.*= *"\(.*\)"/\1/')
echo "==> Releasing v$VERSION"

# Sanity checks
if ! git -C "$SCRIPT_DIR" rev-parse "v$VERSION" &>/dev/null; then
    echo "ERROR: git tag v$VERSION not found. Tag and push it first:"
    echo "  git tag v$VERSION && git push origin v$VERSION"
    exit 1
fi

if [[ ! -d "$AUR_DIR/.git" ]]; then
    echo "ERROR: $AUR_DIR is not a git repo."
    echo "Run: git clone ssh://aur@aur.archlinux.org/presshold.git aur-presshold"
    exit 1
fi

# Copy PKGBUILD and update sha256sums
cp "$SCRIPT_DIR/PKGBUILD" "$AUR_DIR/PKGBUILD"

echo "==> Fetching source tarball and computing sha256sums..."
NEW_SHA=$(cd "$AUR_DIR" && makepkg -g 2>/dev/null | grep -oP "(?<=sha256sums=\(')[a-f0-9]+")
if [[ -z "$NEW_SHA" ]]; then
    echo "ERROR: makepkg -g failed to produce a checksum."
    exit 1
fi
echo "    sha256: $NEW_SHA"
sed -i "s/sha256sums=('[^']*')/sha256sums=('$NEW_SHA')/" "$AUR_DIR/PKGBUILD"

# Also keep the source PKGBUILD in sync
sed -i "s/sha256sums=('[^']*')/sha256sums=('$NEW_SHA')/" "$SCRIPT_DIR/PKGBUILD"

# Generate .SRCINFO and push to AUR
cd "$AUR_DIR"
makepkg --printsrcinfo > .SRCINFO
git add PKGBUILD .SRCINFO
git commit -m "Release v$VERSION"
git push origin master

echo "==> Done. presshold v$VERSION published to AUR."
