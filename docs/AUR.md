# Publishing to the AUR

Step-by-step guide to publish `kio-protondrive` on the Arch User Repository,
so Arch users can `paru -S kio-protondrive` / `yay -S kio-protondrive`
instead of building from source by hand. Unlike
[docs/LAUNCHPAD.md](LAUNCHPAD.md)'s PPA, the AUR is a single split package
(one `pkgbase`, three installable `pkgname`s — `kio-protondrive`,
`kio-protondrive-sync-daemon`, `kio-protondrive-wizard`, mirroring
`debian/control`'s own split) built entirely from source on the *user's own
machine* at install time, not on a central build farm — so, unlike
Launchpad, there is no offline-builder constraint and no vendoring step:
`makepkg` has the same real network access a local dev build does.

The working `PKGBUILD` lives at
[packaging/aur/PKGBUILD](../packaging/aur/PKGBUILD) — built and installed
successfully end to end (`makepkg -si`) in a clean `archlinux:latest`
container as part of developing it. It is **not yet published** — that
needs the one-time AUR account setup below, which (like Launchpad's PPA
creation) requires a browser and can't be scripted.

## 1. One-time setup (manual, on aur.archlinux.org)

1. Create an AUR account at <https://aur.archlinux.org/register> if one
   doesn't already exist.
2. Generate a dedicated SSH key for AUR access (don't reuse another
   project's):

   ```bash
   ssh-keygen -t ed25519 -f ~/.ssh/aur_kio_protondrive -C "aur:kio-protondrive"
   ```

3. On the AUR account's "My Account" page, paste the **public** key
   (`~/.ssh/aur_kio_protondrive.pub`) into the SSH Public Key field.
4. Add an SSH config entry so `ssh` picks the right key for
   `aur.archlinux.org`:

   ```
   Host aur.archlinux.org
     IdentityFile ~/.ssh/aur_kio_protondrive
     User aur
   ```

5. Claim the package by pushing the first commit — the AUR creates the
   package page automatically on first push, there's no separate "create
   package" step:

   ```bash
   git clone ssh://aur@aur.archlinux.org/kio-protondrive.git aur-kio-protondrive
   cp packaging/aur/PKGBUILD aur-kio-protondrive/
   cd aur-kio-protondrive
   makepkg --printsrcinfo > .SRCINFO
   git add PKGBUILD .SRCINFO
   git commit -m "Initial import, v0.9.1"
   git push
   ```

   `.SRCINFO` is AUR's own machine-readable metadata format, generated
   from the `PKGBUILD` by `makepkg` itself — regenerate it after every
   `PKGBUILD` change (step 3 below) and commit both together; the AUR web
   UI rejects a push where they've drifen apart.

## 2. Verify before every push

`makepkg -si` builds *and installs* — do this in a disposable container or
VM, never on a machine you rely on, since a bad `package()` step can put
files anywhere under `/`:

```bash
docker run --rm -it -v "$PWD:/work" archlinux:latest bash
pacman -Syu --needed base-devel cmake extra-cmake-modules rust git pkgconf \
  qt6-base kio kcoreaddons kwidgetsaddons gettext qt6-declarative kirigami sudo
useradd -m builder && echo 'builder ALL=(ALL) NOPASSWD: ALL' > /etc/sudoers.d/builder
cp -r /work/packaging/aur /home/builder/work && chown -R builder:builder /home/builder/work
su builder -c 'cd /home/builder/work && makepkg -si --noconfirm'
```

`checkpkg`/`namcap PKGBUILD` (from the `namcap` package) additionally lint
against common AUR-quality-guideline mistakes — worth running before a
first publish.

## 3. Releasing an update

On every `kio-protondrive` version bump (this repo's own `vX.Y.Z` tags,
driven by `release-please` — see the root `CHANGELOG.md`):

```bash
cd aur-kio-protondrive
git pull
# bump pkgver=, reset pkgrel=1, update sha256sums (see below)
makepkg --printsrcinfo > .SRCINFO
git add PKGBUILD .SRCINFO
git commit -m "Update to vX.Y.Z"
git push
```

`sha256sums` in the committed `packaging/aur/PKGBUILD` is `'SKIP'` — fine
for local testing (this doc's step 2), but the AUR's own quality
guidelines require a real checksum for a published package. Compute it
against the actual release tarball before pushing:

```bash
curl -sL "https://github.com/Aarklendoia/kio-protondrive/archive/refs/tags/vX.Y.Z.tar.gz" | sha256sum
```

A `pkgrel` bump alone (no `pkgver` change) is for a packaging-only fix
(e.g. a `PKGBUILD` correction) that doesn't correspond to a new upstream
release — reset it to `1` whenever `pkgver` changes, increment it
otherwise.

## 4. Automating this (not yet done)

Once the account/SSH key above exist, this can follow
[.github/workflows/publish-ppa.yml](../.github/workflows/publish-ppa.yml)'s
pattern: a workflow triggered on `vX.Y.Z` tags that bumps `pkgver`,
recomputes `sha256sums`, regenerates `.SRCINFO` (needs `makepkg`, so an
`archlinux` container job, not `ubuntu-latest`), and pushes to the AUR git
remote using the SSH private key as a repository secret. Not wired up yet
— nothing to automate against until the package is actually claimed on the
AUR (step 1).

## What the PKGBUILD had to work around

Three issues surfaced only on a real Arch build (none of these affect the
Debian/Launchpad packaging, which doesn't hit them) — documented here so a
future edit doesn't reintroduce them:

- **`corrosion_import_crate` building the whole Cargo workspace.**
  Unrestricted, it also builds `daemon`'s and `wizard`'s bin targets as an
  unused side effect of importing `core/`'s manifest (they're built
  separately either way — see `debian/rules`' own
  `override_dh_auto_build`). Harmless duplication on other toolchains, but
  it went through this project's own from-scratch Corrosion+lld build
  first and tripped the next issue before the real (separate) build even
  got there. Fixed upstream, in `CMakeLists.txt`, by scoping the import
  with `CRATES protondrive-core`.
- **`corrosion_add_cxxbridge`'s linkage to the crate isn't as complete as
  `corrosion_import_crate`'s own imported target.** `protondrive.so` (and
  the other two plugins) only linked `protondrive-core-cxxbridge`, which
  doesn't itself carry `protondrive_core`'s own transitive requirements —
  its native link libraries (rusqlite's bundled sqlite3) in particular.
  Fixed upstream, in `worker/CMakeLists.txt`, by also linking
  `protondrive_core` directly, plus `SQLite::SQLite3` when
  `find_package(SQLite3 QUIET)` finds one (Arch has it; Debian's current
  build environment doesn't, and keeps working without it — see that
  file's own comments for why doing this unconditionally isn't safe
  there) — and by relaxing `-Wl,-z,undefs` on those same three targets,
  since ECM's `KDECompilerSettings`-added `-Wl,--no-undefined` otherwise
  also catches a handful of cxx's own unconditionally-monomorphized
  `CxxVector<T>` symbols for primitive types this project's FFI surface
  never actually uses (genuinely dead code, not a real bug).
- **`makepkg.conf`'s default `CFLAGS`/`CXXFLAGS` (`-flto=auto`) break
  linking a `cargo build`/`cargo test` binary against rusqlite's bundled
  sqlite3.** The `cc` crate (used by `libsqlite3-sys`'s build script)
  honors the ambient `CFLAGS`, so its `sqlite3.o` ends up as LTO bytecode
  — fine for CMake's own C++ link of the same archive (`g++` is invoked
  with matching `-flto` and its LTO plugin resolves it), but not for a
  plain `cargo build`, which links via `rustc`/`cc` with no `-flto` at
  all. This one's packaging-only (`PKGBUILD`'s `_cargo_no_lto` wrapping
  `CFLAGS`/`CXXFLAGS` with `-fno-lto` around the Cargo calls specifically)
  rather than an upstream `CMakeLists.txt`/`Cargo.toml` fix, since
  Debian's build environment doesn't default to LTO and so never hits it.
