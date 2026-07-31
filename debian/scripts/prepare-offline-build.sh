#!/bin/sh
# Run this locally, with network access, before building a Launchpad source
# upload (debuild -S -sa). NEVER run as part of debian/rules — Launchpad's
# build farm has no general internet access, so everything this script
# fetches must already be sitting in the working directory by the time
# dpkg-source tars it up.
#
# It does three things CMake/Corrosion would otherwise do over the network:
#   1. cargo vendor: pulls every crates.io dependency of the workspace
#      (core/, daemon/) into vendor/, and writes .cargo/config.toml to point
#      cargo at it instead of the registry. debian/rules sets
#      CARGO_NET_OFFLINE=true when vendor/ exists, which reaches Corrosion's
#      internal `cargo build` call.
#   2. Also vendors cxxbridge-cmd — the cxxbridge CLI tool Corrosion builds
#      for itself via a separate `cargo install cxxbridge-cmd --version
#      <matching the workspace's resolved cxx crate version> --locked` (see
#      third_party/corrosion/cmake/Corrosion.cmake's
#      _corrosion_check_cxx_version/cxxbridge_v<version> target). That's a
#      completely independent resolve from the workspace's own
#      dependencies — vendoring core/'s and daemon's deps alone does NOT
#      cover it, which is why a real (offline) Launchpad build failed with
#      "failed to select a version for the requirement `clap = ...`
#      (locked to ...)" even though the workspace itself vendored fine (see
#      https://launchpad.net/~aarklendoia-edtech/+archive/ubuntu/kio-protondrive/+build/33437572).
#      Fixed by vendoring a throwaway scratch crate that depends on
#      cxxbridge-cmd at the exact version Corrosion will request, merged
#      into the same vendor/ directory via `cargo vendor --sync`.
#   3. Vendors a copy of Corrosion's own CMake integration into
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
  rm -rf vendor .cargo Cargo.lock

  echo "==> Resolving the workspace to find the exact cxx version Corrosion will need"
  cargo "+$RUST_TOOLCHAIN" generate-lockfile
  CXX_VERSION="$(awk -F'"' '/^name = "cxx"$/{f=1} f && /^version = /{print $2; exit}' Cargo.lock)"
  if [ -z "$CXX_VERSION" ]; then
    echo "    Could not find cxx's resolved version in Cargo.lock" >&2
    exit 1
  fi
  echo "    cxx resolved to $CXX_VERSION — Corrosion will \`cargo install cxxbridge-cmd" \
       "--version $CXX_VERSION --locked\` to match, which vendoring core/'s and daemon's" \
       "own dependencies doesn't cover (it's a separate resolve, not a workspace dependency)"

  # `--locked` makes that `cargo install` use cxxbridge-cmd's OWN published
  # Cargo.lock verbatim rather than re-resolving — so vendoring must supply
  # exactly those pinned versions too, not just anything matching the same
  # semver ranges. A synthetic crate depending on `cxxbridge-cmd = "=$CXX_VERSION"`
  # and letting Cargo re-resolve is NOT equivalent: it can legitimately land
  # on a newer patch release within range (e.g. clap 4.6.4) than what
  # cxxbridge-cmd's own bundled lock actually pins (e.g. clap 4.6.2),
  # which then fails offline with "failed to select a version for the
  # requirement `clap = ...` (locked to 4.6.2), candidate versions found
  # which didn't match: 4.6.4" — hit for real trying to fix this the naive
  # way. So: fetch the real published crate and vendor from its own
  # Cargo.toml + Cargo.lock as-is.
  SCRATCH_DIR="$(mktemp -d)"
  CRATE_UA="kio-protondrive-packaging (https://github.com/Aarklendoia/kio-protondrive)"
  curl -sL -A "$CRATE_UA" \
    "https://crates.io/api/v1/crates/cxxbridge-cmd/$CXX_VERSION/download" \
    -o "$SCRATCH_DIR/cxxbridge-cmd.crate"
  tar -xzf "$SCRATCH_DIR/cxxbridge-cmd.crate" -C "$SCRATCH_DIR"
  CRATE_SRC_DIR="$SCRATCH_DIR/cxxbridge-cmd-$CXX_VERSION"
  if [ ! -f "$CRATE_SRC_DIR/Cargo.lock" ]; then
    echo "    cxxbridge-cmd $CXX_VERSION's published package doesn't bundle a" >&2
    echo "    Cargo.lock — cannot vendor matching --locked's exact pins. Check" >&2
    echo "    https://crates.io/crates/cxxbridge-cmd/$CXX_VERSION" >&2
    exit 1
  fi

  echo "==> Vendoring the workspace + cxxbridge-cmd =$CXX_VERSION (its own pinned deps) into the same vendor/"
  cargo "+$RUST_TOOLCHAIN" vendor vendor --sync "$CRATE_SRC_DIR/Cargo.toml" > /tmp/cargo-vendor-config.toml.tmp
  mkdir -p .cargo
  cat /tmp/cargo-vendor-config.toml.tmp > .cargo/config.toml
  rm -f /tmp/cargo-vendor-config.toml.tmp
  rm -rf "$SCRATCH_DIR"
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
