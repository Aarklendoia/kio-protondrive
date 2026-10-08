#!/bin/sh
# Writes a Gentoo overlay holding kde-misc/kio-protondrive-<version>.ebuild,
# built from the kio-protondrive.ebuild template next to this script.
#
# Usage: packaging/gentoo/make-overlay.sh OUTDIR
#
# Run it on a Gentoo system (or container) with network access: it needs
# cargo, curl, app-portage/pycargoebuild and ::gentoo's license mapping. It
# doesn't write the Manifest, which needs the distfiles: run
# `ebuild OUTDIR/kde-misc/kio-protondrive/*.ebuild manifest` afterwards.
#
# Cargo.lock isn't committed, so this resolves one from the committed tree
# (`git archive HEAD`, so untracked files and a local Cargo.lock are left
# out) and pins its crates in CRATES. It also adds the crates of
# cxxbridge-cmd's own published Cargo.lock, at the cxx version that
# resolve picked: the ebuild builds cxxbridge from them, offline, the same
# way debian/scripts/prepare-offline-build.sh vendors them for the PPA.

set -eu

if [ $# -ne 1 ]; then
  echo "Usage: $0 OUTDIR" >&2
  exit 1
fi
OUTDIR="$1"

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml" | head -1)"
if [ -z "$VERSION" ]; then
  echo "Could not read the workspace version from Cargo.toml" >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "==> Resolving Cargo.lock for kio-protondrive $VERSION"
mkdir "$WORK/src"
git -C "$ROOT" archive HEAD | tar -x -C "$WORK/src"
cargo generate-lockfile --manifest-path "$WORK/src/Cargo.toml"

CXX_VERSION="$(awk -F'"' '/^name = "cxx"$/{f=1} f && /^version = /{print $2; exit}' "$WORK/src/Cargo.lock")"
if [ -z "$CXX_VERSION" ]; then
  echo "Could not find cxx's resolved version in Cargo.lock" >&2
  exit 1
fi

echo "==> Fetching cxxbridge-cmd $CXX_VERSION for its Cargo.lock"
curl -fsSL -A "kio-protondrive-packaging (https://github.com/Aarklendoia/kio-protondrive)" \
  "https://crates.io/api/v1/crates/cxxbridge-cmd/$CXX_VERSION/download" \
  | tar -xz -C "$WORK"
if [ ! -f "$WORK/cxxbridge-cmd-$CXX_VERSION/Cargo.lock" ]; then
  echo "cxxbridge-cmd $CXX_VERSION doesn't ship a Cargo.lock" >&2
  exit 1
fi

PKGDIR="$OUTDIR/kde-misc/kio-protondrive"
EBUILD="$PKGDIR/kio-protondrive-$VERSION.ebuild"
mkdir -p "$PKGDIR" "$OUTDIR/profiles" "$OUTDIR/metadata"
echo kio-protondrive > "$OUTDIR/profiles/repo_name"
cat > "$OUTDIR/metadata/layout.conf" <<EOF
masters = gentoo
thin-manifests = true
sign-manifests = false
EOF
cp "$ROOT/packaging/gentoo/metadata.xml" "$PKGDIR/"
sed "s/@CXXBRIDGE_PV@/$CXX_VERSION/" "$ROOT/packaging/gentoo/kio-protondrive.ebuild" > "$EBUILD"

echo "==> Filling CRATES and the crate licenses"
mkdir "$WORK/distdir"
pycargoebuild --no-config --inplace "$EBUILD" \
  --distdir "$WORK/distdir" \
  "$WORK/src/core" "$WORK/src/daemon" "$WORK/src/wizard" \
  "$WORK/cxxbridge-cmd-$CXX_VERSION"

echo "==> Wrote $EBUILD"
