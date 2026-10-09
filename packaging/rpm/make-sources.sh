#!/bin/sh
# Writes the two sources packaging/rpm/kio-protondrive.spec builds from
# into OUTDIR (#201), under fixed names since the OBS package fetches them
# from the latest GitHub release (packaging/rpm/_service):
#   - kio-protondrive-source.tar.gz: `git archive HEAD`, with a
#     kio-protondrive-<version>/ prefix;
#   - kio-protondrive-vendor.tar.xz: vendor/, .cargo/config.toml,
#     Cargo.lock and third_party/corrosion/ from
#     debian/scripts/prepare-offline-build.sh, i.e. the workspace's crates
#     plus cxxbridge-cmd's (Corrosion builds it at configure time) and
#     Corrosion itself (Fedora 44 only packages 0.5), so the build runs
#     offline as OBS requires.
#
# Usage: packaging/rpm/make-sources.sh OUTDIR
#
# Needs network access, git and rustup (see prepare-offline-build.sh;
# RUST_TOOLCHAIN defaults to stable here, since openSUSE Tumbleweed and
# Fedora ship a recent cargo).

set -eu

if [ $# -ne 1 ]; then
  echo "Usage: $0 OUTDIR" >&2
  exit 1
fi
mkdir -p "$1"
OUTDIR="$(cd "$1" && pwd)"

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml" | head -1)"
P="kio-protondrive-$VERSION"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "==> kio-protondrive-source.tar.gz ($P)"
git -C "$ROOT" archive --format=tar.gz --prefix="$P/" -o "$OUTDIR/kio-protondrive-source.tar.gz" HEAD

echo "==> kio-protondrive-vendor.tar.xz"
mkdir "$WORK/$P"
git -C "$ROOT" archive HEAD | tar -x -C "$WORK/$P"
(cd "$WORK/$P" && RUST_TOOLCHAIN="${RUST_TOOLCHAIN:-stable}" ./debian/scripts/prepare-offline-build.sh > /dev/null)
tar -C "$WORK/$P" -cJf "$OUTDIR/kio-protondrive-vendor.tar.xz" vendor .cargo Cargo.lock third_party/corrosion

ls -l "$OUTDIR/kio-protondrive-source.tar.gz" "$OUTDIR/kio-protondrive-vendor.tar.xz"
