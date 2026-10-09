#!/bin/sh
# Writes the two sources packaging/rpm/kio-protondrive.spec builds from
# into OUTDIR (#201):
#   - kio-protondrive-<version>.tar.gz: `git archive HEAD`;
#   - kio-protondrive-<version>-vendor.tar.xz: vendor/, .cargo/config.toml,
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

echo "==> $P.tar.gz"
git -C "$ROOT" archive --format=tar.gz --prefix="$P/" -o "$OUTDIR/$P.tar.gz" HEAD

echo "==> $P-vendor.tar.xz"
mkdir "$WORK/$P"
git -C "$ROOT" archive HEAD | tar -x -C "$WORK/$P"
(cd "$WORK/$P" && RUST_TOOLCHAIN="${RUST_TOOLCHAIN:-stable}" ./debian/scripts/prepare-offline-build.sh > /dev/null)
tar -C "$WORK/$P" -cJf "$OUTDIR/$P-vendor.tar.xz" vendor .cargo Cargo.lock third_party/corrosion

ls -l "$OUTDIR/$P.tar.gz" "$OUTDIR/$P-vendor.tar.xz"
