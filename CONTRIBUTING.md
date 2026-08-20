# Contributing to kio-protondrive

Thank you for contributing! Here's how to proceed.

## Development Setup

### Prerequisites

- Rust (stable) — for `core/`, no system dependencies beyond a C++17
  compiler (already needed by `build-essential`).
- To also build the actual KIO worker plugin (`worker/`), additionally:
  `cmake extra-cmake-modules qt6-base-dev libkf6kio-dev libkf6coreaddons-dev
  pkg-config build-essential`.

### Setting up the environment

```bash
git clone https://github.com/Aarklendoia/kio-protondrive.git
cd kio-protondrive

# Rust-only: run the core/ unit tests, no KF6/Qt6 needed.
cargo test --manifest-path core/Cargo.toml

# Full build, including the KIO worker plugin:
sudo apt-get install cmake extra-cmake-modules qt6-base-dev \
  libkf6kio-dev libkf6coreaddons-dev pkg-config build-essential
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build
ctest --test-dir build --output-on-failure  # includes worker/tests/'s QTest suite
```

## Contribution Process

### 1. Create a branch

```bash
git checkout -b feature/my-feature
# or
git checkout -b fix/my-fix
```

### 2. Make your changes

- Follow the code style (rustfmt)
- Write tests for new logic in `core/` (see `core/src/cli.rs`'s
  `CommandRunner` mock for the pattern — never spawn the real `proton-drive`
  CLI from a test), or in `worker/tests/` for pure-logic C++ (see
  `worker/tests/tst_shareable.cpp` for the pattern — QTest, no live D-Bus/CLI)
- Update the documentation

### 3. Test your code

```bash
cargo test --manifest-path core/Cargo.toml
cargo clippy --manifest-path core/Cargo.toml --all-targets -- -D warnings
cargo fmt --all
cargo audit
```

### 4. Commit with clear messages

```bash
git commit -m "feat: add rename support"
```

Commit message format:

- `feat:` for new features
- `fix:` for bug fixes
- `docs:` for documentation
- `style:` for style changes
- `refactor:` for refactoring
- `perf:` for performance optimizations
- `test:` for tests
- `chore:` for maintenance tasks

Commit messages drive automated releases (see "Releases" below), so the
prefix matters: `feat:` bumps the minor version, `fix:`/`perf:` bump the
patch version, and a `!` after the type (e.g. `feat!:`) or a `BREAKING
CHANGE:` footer bumps the major version. Everything else (`chore:`,
`style:`, `docs:`, `refactor:`, `test:`, `ci:`) doesn't trigger a release by
itself.

### 5. Push and create a Pull Request

```bash
git push origin feature/my-feature
```

CI will automatically check: Rust tests pass, formatting and clippy are
clean, no known vulnerable dependencies, and the worker plugin still
compiles against KF6::KIOCore (`build-cmake.yml`).

## Debian Packaging

`debian/source/format` is `3.0 (native)` — the upstream version alone is the
package version, no `-1` Debian revision suffix.

To build locally:

```bash
sudo apt-get install debhelper cmake extra-cmake-modules qt6-base-dev \
  libkf6kio-dev libkf6coreaddons-dev
dpkg-buildpackage -us -uc -b -d
```

The generated packages will be in the parent directory. See
[docs/LAUNCHPAD.md](docs/LAUNCHPAD.md) for publishing to the Launchpad PPA.

## Releases

Versioning and releases are automated by
[release-please](https://github.com/googleapis/release-please) — **don't
hand-edit the version in `Cargo.toml` or add a `debian/changelog` entry for
a release.** On every push to `main`, it reads the Conventional Commits
since the last release, maintains an up-to-date "Release PR" that bumps
`Cargo.toml`'s workspace version and accumulates `CHANGELOG.md`. Merging
that PR creates a `vX.Y.Z` git tag and a GitHub Release, which triggers
`build-debian.yml` (builds and attaches `.deb` files) and `publish-ppa.yml`
(uploads a source package to the Launchpad PPA).

## Bug Reports

Create a GitHub issue with:

- Steps to reproduce
- Expected vs actual behavior
- `proton-drive --version` output
- Whether the failure happens via `kioclient5` directly or only in Dolphin

## License

By contributing, you agree that your code will be published under the same
license as the project (GPL-3.0-or-later).
