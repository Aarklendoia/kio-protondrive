# kio-protondrive

[![Tests](https://github.com/Aarklendoia/kio-protondrive/actions/workflows/test.yml/badge.svg)](https://github.com/Aarklendoia/kio-protondrive/actions/workflows/test.yml)
[![Build KIO Worker](https://github.com/Aarklendoia/kio-protondrive/actions/workflows/build-cmake.yml/badge.svg)](https://github.com/Aarklendoia/kio-protondrive/actions/workflows/build-cmake.yml)
[![Build Debian Packages](https://github.com/Aarklendoia/kio-protondrive/actions/workflows/build-debian.yml/badge.svg)](https://github.com/Aarklendoia/kio-protondrive/actions/workflows/build-debian.yml)
[![Quality](https://github.com/Aarklendoia/kio-protondrive/actions/workflows/quality.yml/badge.svg)](https://github.com/Aarklendoia/kio-protondrive/actions/workflows/quality.yml)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3--or--later-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org)

A native KIO worker that lets [Dolphin](https://apps.kde.org/dolphin/) (and
any other KIO-aware KDE application) browse, download, upload and manage
files on [Proton Drive](https://proton.me/drive) directly, via the
`protondrive://` protocol — no separate sync client, no local mirror.

Proton doesn't (yet) ship a native Linux GUI client or a Dolphin
integration — only an [official CLI](https://proton.me/support/drive-cli).
This project wraps that CLI in a real KIO worker, so `protondrive:/my-files`
shows up in Dolphin's location bar and sidebar like any other remote
protocol (`sftp://`, `smb://`, Google Drive's `kio-gdrive`, ...).

## How it works

```text
Dolphin  ──KIO protocol──▶  kio_protondrive (KF6::KIOCore plugin, C++)
                                  │
                                  │  cxx FFI bridge
                                  ▼
                          protondrive-core (Rust)
                                  │
                                  │  subprocess + JSON (-j)
                                  ▼
                          proton-drive CLI  ──▶  Proton Drive
```

- **`core/`** (Rust) — all the actual logic: shells out to the official
  `proton-drive` CLI, parses its JSON output, maps KIO paths to Proton Drive
  paths, and maps CLI errors to something the worker can report. Fully unit
  tested against recorded JSON fixtures (see `core/src/cli.rs`) — no network
  access or live Proton Drive session needed to run `cargo test`.
- **`worker/`** — a thin C++ shim (there's no Rust binding for
  `KIO::WorkerBase`) implementing the KIO protocol methods
  (`listDir`/`stat`/`get`/`put`/`mkdir`/`del`) and calling into `core/`
  through a [`cxx`](https://cxx.rs) bridge.
- Built with [Corrosion](https://github.com/corrosion-rs/corrosion), which
  drives the Rust build from CMake and links the resulting static library
  into the worker plugin.

Files are fetched **on demand** when opened (like `sftp://`), not
synchronized to a local folder in the background — see
[docs/DESIGN.md](docs/DESIGN.md) for why a sync daemon is a deliberately
separate concern.

## Pinning files for offline/instant access

Everything under `protondrive:/` is fetched on demand by default — nothing
is kept locally. If you want a specific file or folder to always be
available instantly (no download wait) and offline, right-click it in
Dolphin and choose **Garder en local** ("Keep it local"). This downloads a
local copy that the KIO worker then serves straight from disk for `get`/
`stat`, no CLI round-trip. **Supprimer la copie locale** ("Remove the local
copy") un-pins it again — the local copy is deleted, the file on Drive is
untouched.

A separate package, `kio-protondrive-sync-daemon`, provides the background
piece this needs — install `kio-protondrive-full` to get both it and the
KIO worker. It's a `systemd --user` service that does two things: runs the
`pin`/`unpin` action requested from Dolphin's context menu, and watches
already-pinned files for local edits, uploading changed ones back to Drive
automatically.

**Scope**: one-way local → Drive upload for *pinned* files only. Picking up
changes made to a pinned file from elsewhere (another device, the web app)
happens on next access, not via continuous background polling — see
[docs/DESIGN.md](docs/DESIGN.md) and
[#30](https://github.com/Aarklendoia/kio-protondrive/issues/30) for the
full design.

No configuration file is required to get started. Enable and start it:

```console
$ systemctl --user enable --now kio-protondrive-sync-daemon.service
$ journalctl --user -u kio-protondrive-sync-daemon -f
```

### Credential persistence

By default, the `proton-drive` CLI keeps its session in the desktop's Secret
Service (`libsecret`/GNOME Keyring/KWallet). That's fine for the KIO worker,
which only ever runs inside an already-unlocked Dolphin session, but it's
the wrong fit for a `systemd --user` service that must authenticate with no
one present to unlock a keyring — and on some setups (e.g. a Secret Service
provider with no persistent on-disk collection) the session silently fails
to survive a keyring-daemon restart at all, forcing a fresh
`proton-drive auth login` every time.

To avoid depending on the keyring, the packaged daemon service sets
`PROTON_DRIVE_CREDENTIALS_STORE=unsafe_file` (an env var the CLI itself
supports, undocumented in `--help` but present in its own source), which
persists the session to a plain file at
`~/.local/share/proton-drive-cli/auth-session.json` instead — created with
`0600` permissions (readable only by you), but **not encrypted at rest**,
unlike the keyring. If you'd rather use `pass`
(GPG-encrypted) instead, override it:

```console
$ systemctl --user edit kio-protondrive-sync-daemon.service
```

```ini
[Service]
Environment=PROTON_DRIVE_CREDENTIALS_STORE=pass
```

(`pass` requires `pass` and a GPG key already set up, and `gpg-agent` able
to decrypt without an interactive prompt for the daemon to authenticate
unattended.) The KIO worker is unaffected either way — it keeps using
whichever store the CLI's own default (`keychain`) or your shell
environment already selects.

## Scope

**Supported (v1):**

- Browsing folders, including Proton Drive's virtual top-level sections
  (`/my-files`, `/devices`, `/shared-with-me`, `/trash`, ...)
- Opening/downloading files, uploading/overwriting files, creating folders
- Deleting a file or folder (moves it to Proton Drive's own trash — there is
  no permanent delete exposed through Dolphin in v1)
- Browsing `/photos`, read-only: `filesystem list`/`info` genuinely don't
  support Photos (see "Blocked upstream" below), but CLI 0.7.0+ exposes it
  through a separate, non-path-based `photo` command family instead, which
  this worker wraps into a flat, real-filename listing. No upload (the CLI's
  own `photo upload` always lands in "My Photos", flat, regardless of
  destination) and no albums. See [#18](https://github.com/Aarklendoia/kio-protondrive/issues/18).
- A persistent listing/stat cache, so repeat browsing (and Dolphin's
  breadcrumb, which stats every path segment on its own) is instant instead
  of re-hitting the CLI every time — kept fresh by the sync daemon's
  periodic sweep and by this app's own writes, not by a fixed expiry. See
  `docs/DESIGN.md`'s "Filesystem listing/stat cache" section for the
  consistency tradeoff this accepts. See [#8](https://github.com/Aarklendoia/kio-protondrive/issues/8).
- Cancellable uploads/downloads (Dolphin's Cancel button actually stops the
  transfer), with an approximate, time-based progress bar rather than
  real byte-accurate progress — the `proton-drive` CLI has no stable
  progress API to report real numbers from. See `docs/DESIGN.md`'s
  "Cancellable transfers and approximate progress" section.
  See [#9](https://github.com/Aarklendoia/kio-protondrive/issues/9).
- An opportunistic local file cache: any file you open (or save) stays
  available locally afterward instead of being deleted immediately, so
  reopening it is instant — until it's evicted after a configurable number
  of days since last use (30 by default, set during setup). Pinned files
  ("Garder en local") are never evicted this way. Dolphin shows up to two
  status badges per item — a checkmark for "available locally" (pinned or
  cached) and a pin icon on top specifically for pinned files, OneDrive-
  style. See `docs/DESIGN.md`'s "Opportunistic local file cache" section for
  how cache hits stay correct (unlike pinning, a hit here is re-verified
  against the remote before being trusted). See
  [#60](https://github.com/Aarklendoia/kio-protondrive/issues/60).

**Not yet implemented** (contributions welcome):

- Server-side copy (KIO falls back to download+upload, which works but is
  slower) — rename/move are implemented ([#5](https://github.com/Aarklendoia/kio-protondrive/issues/5))
- Sharing/invitations
- Browsing Proton Drive's trash as a restorable Dolphin trash view
- Albums, uploading to Photos ([#18](https://github.com/Aarklendoia/kio-protondrive/issues/18))

**Blocked upstream:** `/albums` and the `photos-shared-by-me`/
`photos-shared-with-me`/`photos-trash` sections shown when listing `/` fail
every operation with "Path type ... is not supported" from the
`proton-drive` CLI itself — this isn't something a KIO worker can work
around. See [#18](https://github.com/Aarklendoia/kio-protondrive/issues/18).

**Known limitation: no thumbnails.** Dolphin/KIO's `PreviewJob` enforces a
hard, non-configurable 2-second timeout per file (`startTimer(2s)` in KDE
Frameworks' `filepreviewjob.cpp`, confirmed against the installed KF6 6.24.0
sources). Every `proton-drive` CLI invocation carries 1.2-4.3s of its own
fixed overhead regardless of file size — confirmed live with `time
proton-drive filesystem info` (1.2s) and `time proton-drive photo download`
(1.8-4.3s for files under 2.5 MB); `user`+`sys` account for well under a
second of that, so the rest is the CLI's own Node.js/SDK session startup on
every call, not network transfer. No caching or worker-side optimization can
close a gap this structural: the CLI alone routinely takes longer than
Dolphin allows for the entire preview attempt, so every thumbnail request
times out before the download can even finish. Fixing this for real would
mean keeping a long-lived, already-authenticated `proton-drive` session warm
across calls (a background daemon brokering CLI access instead of spawning
it fresh per operation) — a much larger architecture change than this
project's current on-demand, stateless design (see `docs/DESIGN.md`).
Separately, and independently of the timeout, HEIC photos (most iPhone
photos) have no working thumbnailer on Kubuntu 26.04 at all regardless of
protocol: the system's only HEIF/AVIF thumbnailer (`glycin-heif`) ships as an
external freedesktop.org `.thumbnailer` rather than a native KDE
`ThumbCreator` plugin, and isn't picked up by KIO's plugin loader in this
Frameworks version — a distro packaging gap, not something specific to
Proton Drive.

## Installing

Requires the official [Proton Drive CLI](https://proton.me/drive/download)
to already be installed and logged in (`proton-drive auth login`) — this
package only adds the Dolphin/KIO integration on top of it.

Proton doesn't provide an apt/deb repository or any auto-update mechanism
for that CLI — it's a manually-downloaded binary, so `apt upgrade` won't
update it. Periodically re-check the
[download page](https://proton.me/drive/download) for new versions
yourself; see [#26](https://github.com/Aarklendoia/kio-protondrive/issues/26)
for the tracking issue on whether this project should do more here (e.g.
version-checking) than just this note.

**Ubuntu 26.04 LTS (resolute)**, via the Launchpad PPA:

```bash
sudo add-apt-repository ppa:aarklendoia-edtech/kio-protondrive
sudo apt update
sudo apt install kio-protondrive
```

**Other Debian/Ubuntu versions**: download the `.deb` from the
[Releases page](https://github.com/Aarklendoia/kio-protondrive/releases/latest):

```bash
sudo apt install ./kio-protondrive_*.deb
```

Then open `protondrive:/` in Dolphin's location bar (or
`kioclient5 ls protondrive:/` from a terminal) to browse your Drive.

### Pinning it to Dolphin's sidebar

Browse to `protondrive:/`, then right-click the breadcrumb (or drag it into
the **Places** panel) and choose **Add to Places**. This is standard
Dolphin/KIO behavior, not something this package sets up for you — but once
bookmarked, Dolphin shows it as **Proton Drive** (with a cloud icon) under
the *Remote* section of the sidebar, and uses that name in the breadcrumb
and window title instead of the raw `protondrive:/` URL — the same way it
does for any other bookmarked network location.

Bookmarking the root (rather than `protondrive:/my-files` directly) is
deliberate: Dolphin's Places panel has no concept of a bookmark with nested
children (every entry is a flat leaf — see
[KFilePlacesModel](https://github.com/KDE/kio/blob/master/src/filewidgets/kfileplacesmodel.h)),
so there's no way to mirror the Proton Drive web UI's sidebar sections
(My files, Computers, Shared, ...) as an actual tree in Dolphin's sidebar.
One click into the **Proton Drive** bookmark gets you a folder listing of
those same sections instead — the closest equivalent Dolphin supports.

## Building from source

```bash
sudo apt-get install build-essential cmake extra-cmake-modules pkg-config \
  qt6-base-dev libkf6kio-dev libkf6coreaddons-dev
cargo build --manifest-path core/Cargo.toml   # Rust core only, for quick iteration
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build
cmake --install build --prefix "$HOME/.local"  # test without root
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full development workflow,
and [docs/LAUNCHPAD.md](docs/LAUNCHPAD.md) for the Debian/Launchpad release
process.

## License

[GPL-3.0-or-later](LICENSE).
