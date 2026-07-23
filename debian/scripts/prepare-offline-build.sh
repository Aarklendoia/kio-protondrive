#!/bin/sh
# Run this locally, with network access, before building a Launchpad source
# upload (debuild -S -sa). NEVER run as part of debian/rules — Launchpad's
# build farm has no general internet access, so everything this script
# fetches must already be sitting in the working directory by the time
# dpkg-source tars it up.
#
# It does two things CMake/Corrosion would otherwise do over the network:
#   1. cargo vendor: pulls every crates.io dependency of core/ into vendor/,
#      and writes .cargo/config.toml to point cargo at it instead of the
#      registry. debian/rules sets CARGO_NET_OFFLINE=true when vendor/
#      exists, which reaches Corrosion's internal `cargo build` call.
#   2. Vendors a copy of Corrosion's own CMake integration into
#      third_party/corrosion/ — CMakeLists.txt falls back to
#      add_subdirectory(third_party/corrosion) instead of FetchContent-ing
#      it from GitHub when that directory is present.
#
# IMPORTANT: vendor with the SAME cargo version the target Ubuntu series
# ships (check with `rmadison -u ubuntu cargo`), not whatever's locally
# "stable" — a newer cargo vendoring the tree can silently omit
# Cargo.toml.orig companion files an older cargo needs at build time to
# verify a vendored crate's checksum against Cargo.lock (see linux-hello's
# debian/scripts/prepare-offline-build.sh, which hit this for real). Set
# RUST_TOOLCHAIN to the version to vendor with; defaults to "stable", which
# is very likely wrong for an older LTS target — pass the real one
# explicitly:
#   RUST_TOOLCHAIN=1.93.1 ./debian/scripts/prepare-offline-build.sh
#
# See docs/LAUNCHPAD.md for how this fits into the release process.
#
# Set SKIP_CARGO_VENDOR=1 to only vendor Corrosion and leave Cargo
# dependencies untouched — for CI environments that DO have network access
# (e.g. GitHub Actions' build-debian.yml) but still need Corrosion
# pre-populated because dh_auto_configure (debhelper compat 13) always
# passes -DFETCHCONTENT_FULLY_DISCONNECTED=ON regardless of real
# connectivity. Never set this for an actual Launchpad upload.

set -eu

RUST_TOOLCHAIN="${RUST_TOOLCHAIN:-stable}"
CORROSION_TAG="${CORROSION_TAG:-v0.6.0}"
SKIP_CARGO_VENDOR="${SKIP_CARGO_VENDOR:-0}"

cd "$(dirname "$0")/../.."
REPO_ROOT="$(pwd)"

echo "==> Removing build/ (must not exist when dpkg-source tars up the tree)"
rm -rf build

if [ "$SKIP_CARGO_VENDOR" = "1" ]; then
  echo "==> Skipping Cargo dependency vendoring (SKIP_CARGO_VENDOR=1)"
else
  echo "==> Vendoring Cargo dependencies into vendor/ (toolchain: $RUST_TOOLCHAIN)"
  if [ "$RUST_TOOLCHAIN" = "stable" ]; then
    echo "    WARNING: no RUST_TOOLCHAIN given, using 'stable' — check the target" >&2
    echo "    series' cargo version first (rmadison -u ubuntu cargo) and pass" >&2
    echo "    RUST_TOOLCHAIN=<version> explicitly if it differs." >&2
  fi
  rustup toolchain install "$RUST_TOOLCHAIN" > /dev/null 2>&1 || true
  rm -rf vendor .cargo
  cargo "+$RUST_TOOLCHAIN" vendor vendor > /tmp/cargo-vendor-config.toml.tmp
  mkdir -p .cargo
  cat /tmp/cargo-vendor-config.toml.tmp > .cargo/config.toml
  rm -f /tmp/cargo-vendor-config.toml.tmp
  echo "    $(du -sh vendor | cut -f1) in vendor/, $(find vendor -name '*.orig' | wc -l) .orig files"

  echo "==> Disabling cargo's per-file checksum verification for vendored crates"
  # dpkg-source's native-tarball builder always drops files matching a
  # hardcoded exclude list (*.orig, .gitignore, ...), regardless of
  # debian/source/options — some vendored crate, somewhere, will have a test
  # fixture or metadata file matching one of those generic names, and losing
  # it from the tarball breaks cargo's per-file checksum verification for
  # that crate. The documented Debian Rust-packaging fix is to blank out each
  # vendored crate's "files" checksum map so cargo only trusts the vendor
  # directory as-is (the "package" checksum, verified against Cargo.lock, is
  # untouched). Same fix as linux-hello's prepare-offline-build.sh.
  find vendor -maxdepth 2 -name ".cargo-checksum.json" |
    while IFS= read -r f; do
      jq '.files = {}' "$f" > "$f.tmp" && mv "$f.tmp" "$f"
    done
fi

echo "==> Vendoring Corrosion ($CORROSION_TAG) into third_party/corrosion/"
rm -rf third_party/corrosion
mkdir -p third_party
git clone --depth 1 --branch "$CORROSION_TAG" https://github.com/corrosion-rs/corrosion.git third_party/corrosion
rm -rf third_party/corrosion/.git

if [ "$SKIP_CARGO_VENDOR" = "1" ]; then
cat <<EOF

Ready (Corrosion only — SKIP_CARGO_VENDOR=1). CMakeLists.txt will use the
vendored Corrosion copy in third_party/corrosion/ instead of fetching it;
Cargo dependencies are untouched, so cargo/Corrosion's internal cargo build
still needs real network access.
EOF
else
cat <<EOF

Ready. From this same working directory (with vendor/, .cargo/config.toml
and third_party/corrosion/ populated), debian/rules will build with
CARGO_NET_OFFLINE=true and CMakeLists.txt will use the vendored Corrosion
copy instead of fetching it. Proceed with the dch / debuild -S -sa / dput
cycle from docs/LAUNCHPAD.md.

Nothing here is meant to be committed to git — vendor/, .cargo/ and
third_party/corrosion/ are regenerated per release right before packaging
(see .gitignore).
EOF
fi
