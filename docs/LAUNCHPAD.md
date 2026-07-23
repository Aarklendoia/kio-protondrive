# Publishing to Launchpad (PPA)

Step-by-step guide to publish kio-protondrive as a Launchpad PPA
(`ppa:aarklendoia-edtech/kio-protondrive`), so users can `apt install` it
directly instead of downloading `.deb` files from GitHub Releases. This
mirrors the [linux-hello](https://github.com/Aarklendoia/linux-hello) project's
process (see its `docs/LAUNCHPAD.md`) — same Launchpad account, deliberately
a **separate PPA** so the two unrelated projects don't share one archive.

- Launchpad account: `aarklendoia-edtech` (already exists, reused from
  linux-hello)
- Personal signing key (manual uploads): reuse the existing
  `86EB1CE672402B0B104049C3D4251A0893FE3895` (`aarklendoia@proton.me`),
  already confirmed on the account with the Code of Conduct signed — no new
  personal key needed.
- CI signing key (automated uploads): generated and registered on the
  `aarklendoia-edtech` account, fingerprint
  `D61B393270CB976FBF38147EE8A85F33C5CF96B1` — a new, project-specific key,
  deliberately not reusing linux-hello's CI key, so a leaked secret in one
  repo's Actions only compromises that one PPA. Still need to add the
  private key as the `PPA_GPG_PRIVATE_KEY` repository secret (see
  [Automated publishing](#4-automated-publishing-ci) below).
- PPA: `ppa:aarklendoia-edtech/kio-protondrive` — **not created yet**, see
  [step 1](#1-one-time-setup-manual-on-launchpadnet).

Launchpad's build farm has no general internet access, so a plain `cmake
--build` (which triggers Corrosion's `cargo build` and, unmodified, a
`FetchContent` clone of Corrosion itself) would fail there — see
[Vendoring](#2-vendoring-required-before-every-ppa-upload).

## 1. One-time setup (manual, on launchpad.net)

None of this can be automated from a script — it requires a browser and your
Launchpad identity.

1. Create the PPA: your Launchpad profile page
   (`https://launchpad.net/~aarklendoia-edtech`) → "Create a new PPA" → name
   it `kio-protondrive`, public visibility.
2. Generate a new, dedicated CI signing key (don't reuse linux-hello's):

   ```bash
   gpg --full-generate-key   # no passphrase — needed for unattended CI signing
   gpg --list-secret-keys --keyid-format long
   gpg --keyserver keyserver.ubuntu.com --send-keys <NEW_FINGERPRINT>
   ```

3. On your Launchpad profile → "OpenPGP keys" → import the new fingerprint
   as an *additional* key on the same `aarklendoia-edtech` account. Launchpad
   emails a confirmation you must decrypt (`gpg --decrypt`) and follow the
   link in.
4. Install the upload tooling locally, if not already present:

   ```bash
   sudo apt install devscripts dput debhelper lintian gnupg
   ```

5. Update the placeholder in
   [.github/workflows/publish-ppa.yml](../.github/workflows/publish-ppa.yml)
   (`GPG_KEY_ID: 'REPLACE_ME_SEE_DOCS_LAUNCHPAD_MD'`) with the new
   fingerprint, and add the private key as the `PPA_GPG_PRIVATE_KEY` GitHub
   Actions repository secret:

   ```bash
   gpg --armor --export-secret-keys <NEW_FINGERPRINT>
   # paste the output into the GitHub repo secret
   ```

## 2. Vendoring (required before every PPA upload)

Launchpad's builders can't reach crates.io or GitHub during a build. Two
things in this repo assume network access unmodified:

1. **Cargo dependencies** — Corrosion's internal `cargo build` (triggered by
   `cmake --build` during `dh_auto_build`) fetches crates from crates.io.
2. **Corrosion itself** — `CMakeLists.txt` normally `FetchContent`-clones
   `corrosion-rs/corrosion` from GitHub at configure time.

Both work fine on GitHub Actions and locally. The fix, needed only for a PPA
build, is to vendor everything first, from a machine with network access:

```bash
# Check the target series' packaged cargo version first:
rmadison -u ubuntu cargo | grep resolute

RUST_TOOLCHAIN=1.93.1 ./debian/scripts/prepare-offline-build.sh
```

This vendors Cargo dependencies into `vendor/` (+ `.cargo/config.toml`) and
clones a pinned Corrosion release into `third_party/corrosion/` — see the
script's comments for **why the toolchain version matters** (a newer local
cargo can vendor a tree an older, series-packaged cargo can't verify —
already hit and documented in linux-hello's equivalent script). `vendor/`,
`.cargo/` and `third_party/corrosion/` are git-ignored: regenerated per
release, not part of normal `main` history. Since `debian/source/format` is
`3.0 (native)`, `debuild -S` tars up whatever is physically present at that
moment, `.gitignore` notwithstanding.

## 3. Building and uploading a release

One **source-only** upload per target Ubuntu series:

```bash
dch --newversion "0.1.0~ppa1~resolute1" --distribution resolute --urgency medium \
  "Automated PPA build for resolute, release 0.1.0."

debuild -S -sa -k<FINGERPRINT>

dput ppa:aarklendoia-edtech/kio-protondrive ../kio-protondrive_0.1.0~ppa1~resolute1_source.changes
```

Track build status at
`https://launchpad.net/~aarklendoia-edtech/+archive/ubuntu/kio-protondrive/+packages`.

## 4. Automated publishing (CI)

[.github/workflows/publish-ppa.yml](../.github/workflows/publish-ppa.yml)
automates the cycle above on every `vX.Y.Z` release tag (same trigger as
`build-debian.yml`), or manually via `workflow_dispatch`. It won't work
until [step 1](#1-one-time-setup-manual-on-launchpadnet) above is done (the
PPA must exist, and the placeholder `GPG_KEY_ID` / `PPA_GPG_PRIVATE_KEY`
secret must be filled in).

## 5. Once published

Add the `add-apt-repository ppa:aarklendoia-edtech/kio-protondrive` /
`apt install kio-protondrive` instructions to the README's install section
(already there, pointing at this PPA — nothing to change once it's live)
and a Launchpad badge to the badge row.
