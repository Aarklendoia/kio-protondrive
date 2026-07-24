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

## Scope

**Supported (v1):**

- Browsing folders, including Proton Drive's virtual top-level sections
  (`/my-files`, `/devices`, `/shared-with-me`, `/trash`, ...)
- Opening/downloading files, uploading/overwriting files, creating folders
- Deleting a file or folder (moves it to Proton Drive's own trash — there is
  no permanent delete exposed through Dolphin in v1)

**Not yet implemented** (contributions welcome):

- Server-side rename/copy/move (KIO falls back to download+upload, which
  works but is slower)
- Sharing/invitations
- Browsing Proton Drive's trash as a restorable Dolphin trash view
- Directory listing cache, thumbnails

**Blocked upstream:** the Photos section (`/photos`, `/albums` and their
`-shared-by-me`/`-shared-with-me`/`-trash` variants) shows up when listing
`/`, but every operation against it fails with "Path type photos is not
supported" from the `proton-drive` CLI itself — this isn't something a KIO
worker can work around. See [#18](https://github.com/Aarklendoia/kio-protondrive/issues/18).

## Installing

Requires the official [Proton Drive CLI](https://proton.me/drive/download)
to already be installed and logged in (`proton-drive auth login`) — this
package only adds the Dolphin/KIO integration on top of it.

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
