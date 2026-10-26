# kio-protondrive

[![Tests](https://github.com/Aarklendoia/kio-protondrive/actions/workflows/test.yml/badge.svg)](https://github.com/Aarklendoia/kio-protondrive/actions/workflows/test.yml)
[![Build KIO Worker](https://github.com/Aarklendoia/kio-protondrive/actions/workflows/build-cmake.yml/badge.svg)](https://github.com/Aarklendoia/kio-protondrive/actions/workflows/build-cmake.yml)
[![Build Debian Packages](https://github.com/Aarklendoia/kio-protondrive/actions/workflows/build-debian.yml/badge.svg)](https://github.com/Aarklendoia/kio-protondrive/actions/workflows/build-debian.yml)
[![Quality](https://github.com/Aarklendoia/kio-protondrive/actions/workflows/quality.yml/badge.svg)](https://github.com/Aarklendoia/kio-protondrive/actions/workflows/quality.yml)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3--or--later-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org)

**Proton Drive in Dolphin.** Browse, open, upload and manage your
[Proton Drive](https://proton.me/drive) files directly from
[Dolphin](https://apps.kde.org/dolphin/) and any other KDE application,
through the `protondrive:/` address — like `sftp://`, `smb://` or Google
Drive's `kio-gdrive`.

Proton doesn't ship a Linux desktop client or a Dolphin integration (yet),
only an [official command-line tool](https://proton.me/support/drive-cli).
kio-protondrive builds on that CLI, so Proton Drive shows up in Dolphin's
location bar and sidebar like any other network location: no separate sync
app, no full local mirror of your Drive.

> [!NOTE]
> This is an independent community project, **not affiliated with or
> endorsed by Proton AG**. It drives Proton's official `proton-drive` CLI and
> never handles your password or encryption keys itself. Proton describes
> its CLI and SDK as not yet stable for third-party use, so a CLI update can
> occasionally break something here until a fix is released.

## Quick start

On **Ubuntu 26.04 LTS or 26.10** (other distributions:
[Installing](#installing)):

```bash
sudo add-apt-repository ppa:aarklendoia-edtech/kio-protondrive
sudo apt update
sudo apt install kio-protondrive-full
```

Then run **kio-protondrive-wizard** from your application menu (it also
opens by itself the first time it's needed). It installs the Proton Drive
CLI if it's missing, signs you in, and can add **Proton Drive** to
Dolphin's Places panel. Once that's done, open `protondrive:/` in Dolphin.

## Features

- **Browse and manage files**: your files and Proton Drive's own sections
  (My files, Computers, Shared, Shared with me, Trash). Open, save, upload,
  create folders, rename and move. Copies inside Proton Drive happen on the
  server, without a round trip through your disk.
- **Trash**: deleting moves items to Proton Drive's trash. Right-click a
  trashed item to **Restore** it, or the Trash itself to **Empty Trash**.
- **Fast repeat browsing**: folder contents are cached, so going back to a
  folder is instant. Changes made elsewhere (the web app, another device)
  show up a few seconds after you open or refresh (F5) the folder.
- **Files stay available after opening**: an opened or saved file is kept
  locally, so reopening it is instant. It is re-checked against Drive first,
  and removed after a configurable number of days unused (30 by default).
- **Keep Available Offline**: right-click a file or folder to pin a local
  copy that is never evicted and works offline. Edits to pinned files are
  uploaded back to Drive automatically. Status badges show what's available
  locally (checkmark) and what's pinned (star).
- **Sharing**: **Share via Proton Drive** in the context menu manages
  members, invitations and public links. Shared items get a badge.
- **Photos**: `/photos` is browsable (read-only), and **Filter Photos**
  narrows it to favorites, screenshots, videos, selfies, panoramas, RAW...
- **Cancellable transfers**: Dolphin's Cancel button really stops an upload
  or download (the progress bar is an estimate, see
  [Limitations](#limitations)).
- **CLI updates**: you're notified when a new Proton Drive CLI is out, and
  can install it in one click, checksum-verified.

## Installing

Requires the official [Proton Drive CLI](https://proton.me/drive/download),
version 0.9.0 or newer. The setup wizard (in `kio-protondrive-full`)
installs it for you into `~/.local/bin` if it's missing or too old, and
signs you in. With the bare `kio-protondrive` package, install the CLI and
run `proton-drive auth login` yourself first.

The packages come in three parts, the same on every distribution:

| Package | What it does |
| --- | --- |
| `kio-protondrive` | The Dolphin integration itself (the `protondrive:/` KIO worker). |
| `kio-protondrive-sync-daemon` | Background service: offline pinning, local cache cleanup, keeping folder views fresh, CLI update checks. |
| `kio-protondrive-wizard` | Setup wizard: CLI install, sign-in, cache settings, Places entry. |

Installing all three is recommended (`kio-protondrive-full` on Debian and
Ubuntu).

### Ubuntu 26.04 LTS (resolute) and 26.10 (stonking)

From the Launchpad PPA, which also brings updates through `apt upgrade`:

```bash
sudo add-apt-repository ppa:aarklendoia-edtech/kio-protondrive
sudo apt update
sudo apt install kio-protondrive-full
```

`kio-protondrive-full` pulls in all three packages.

Each release also carries the same packages as `.deb` files, built on
Ubuntu 26.04 (on 26.10, prefer the PPA, whose packages are built there).
To install them without the PPA, download the four files
without `+deb13` in their name from the
[Releases page](https://github.com/Aarklendoia/kio-protondrive/releases/latest)
and install them together, in a single command (`apt` only resolves the
packages' dependencies on each other when they're given together):

```bash
sudo apt install ./kio-protondrive_*.deb ./kio-protondrive-sync-daemon_*.deb \
  ./kio-protondrive-wizard_*.deb ./kio-protondrive-full_*.deb
```

Updates are then manual: install the next release's `.deb` files the same
way.

### Debian 13 (trixie)

Each release from 0.14.0 on carries packages built on Debian 13, with
`+deb13` in their version: download the four `*+deb13_*.deb` files from the
[Releases page](https://github.com/Aarklendoia/kio-protondrive/releases/latest)
and install them together, as above:

```bash
sudo apt install ./kio-protondrive_*+deb13_amd64.deb \
  ./kio-protondrive-sync-daemon_*+deb13_amd64.deb \
  ./kio-protondrive-wizard_*+deb13_amd64.deb ./kio-protondrive-full_*+deb13_all.deb
```

Updates are manual: install the next release's files the same way.

### Other Debian and Ubuntu versions

There's no package for them: the Ubuntu packages need Qt, glibc and
libstdc++ from 26.04 or 26.10, and Ubuntu releases before 25.10 have no KDE
Frameworks 6 at all. Build from source instead, see
[Building from source](#building-from-source).

### Arch Linux

The package isn't on the AUR yet — AUR account registration is closed for
now, it'll be published there once it reopens. In the meantime, each
release from 0.11.1 on carries prebuilt packages: download the
`.pkg.tar.zst` files from the
[Releases page](https://github.com/Aarklendoia/kio-protondrive/releases/latest)
and install them together:

```bash
sudo pacman -U kio-protondrive-*.pkg.tar.zst
```

Or build them yourself from the repository's own `PKGBUILD` (the same one
the AUR will get):

```bash
git clone https://github.com/Aarklendoia/kio-protondrive.git
cd kio-protondrive/packaging/aur
makepkg -si
```

Updates are manual until the AUR: repeat either with the next release.

### Gentoo

From the [Aarklendoia/gentoo-overlay](https://github.com/Aarklendoia/gentoo-overlay)
overlay, which each release updates and `emaint sync` (or `emerge --sync`)
then brings in:

```bash
sudo eselect repository add aarklendoia git https://github.com/Aarklendoia/gentoo-overlay.git
sudo emaint sync -r aarklendoia
printf '%s\n' 'kde-misc/kio-protondrive ~amd64' 'dev-build/corrosion ~amd64' \
  | sudo tee /etc/portage/package.accept_keywords/kio-protondrive
sudo emerge --ask kde-misc/kio-protondrive
```

The `daemon` and `wizard` USE flags (both on by default) match the Debian
`kio-protondrive-sync-daemon` and `kio-protondrive-wizard` packages. Each
release also carries the same overlay as
`kio-protondrive-gentoo-overlay-*.tar.gz`, for systems that can't sync
from git.

### openSUSE Tumbleweed and Fedora

Each release from 0.13.0 on carries prebuilt RPMs: download the three
`.rpm` files for your distribution from the
[Releases page](https://github.com/Aarklendoia/kio-protondrive/releases/latest)
(`.tumbleweed` for openSUSE Tumbleweed, `.fc44` for Fedora 44) and install
them together:

```bash
# openSUSE Tumbleweed (the RPMs aren't signed yet)
sudo zypper install --allow-unsigned-rpm ./kio-protondrive-*.tumbleweed.x86_64.rpm
# Fedora 44
sudo dnf install ./kio-protondrive-*.fc44.x86_64.rpm
```

`kio-protondrive-sync-daemon` and `kio-protondrive-wizard` match the
Debian packages of the same names. Updates are manual for now: install the
next release's RPMs the same way.

### Other distributions

Build from source, see [Building from source](#building-from-source).

### After installing

On Arch, Gentoo, openSUSE and Fedora (and when building from source),
nothing enables the sync daemon for you, unlike the Debian packages. Run
this once:

```bash
systemctl --user enable --now kio-protondrive-sync-daemon.service
```

Then run `kio-protondrive-wizard` if you haven't yet, and open
`protondrive:/` in Dolphin (or `kioclient ls protondrive:/` from a
terminal).

## Using it

- **Getting there**: type `protondrive:/` in Dolphin's location bar, or use
  the **Proton Drive** entry in the Places panel (the wizard adds it, or
  right-click the breadcrumb at `protondrive:/` and choose **Add to
  Places**). It sits under *Remote*, with a cloud icon.
- **Context menu actions** (right-click in a `protondrive:/` folder):
  **Keep Available Offline** / **Remove Local Copy**, **Share via Proton
  Drive**, **Filter Photos** (in `/photos`), **Restore** (in the trash) and
  **Empty Trash** (on the trash itself, asks for confirmation).
- **Seeing changes made elsewhere**: open or refresh (F5) the folder;
  Dolphin updates the view a few seconds later. A folder left open without
  touching it is refreshed by the sync daemon every 15 minutes
  ([#225](https://github.com/Aarklendoia/kio-protondrive/issues/225) tracks
  doing better).
- **Sync daemon logs**:
  `journalctl --user -u kio-protondrive-sync-daemon -f`.

<details>
<summary>Why the Places entry is the Drive root, not "My files"</summary>

Dolphin's Places panel has no concept of a bookmark with nested children
(every entry is a flat leaf — see
[KFilePlacesModel](https://github.com/KDE/kio/blob/master/src/filewidgets/kfileplacesmodel.h)),
so there's no way to mirror the Proton Drive web UI's sidebar sections (My
files, Computers, Shared, ...) as an actual tree in Dolphin's sidebar. One
click into the **Proton Drive** bookmark gets you a folder listing of those
same sections instead — the closest equivalent Dolphin supports. Once
bookmarked, Dolphin also uses the name **Proton Drive** in the breadcrumb and
window title instead of the raw `protondrive:/` URL.

</details>

## Limitations

- **No thumbnails.** Dolphin gives each preview 2 seconds, and every
  `proton-drive` CLI call alone takes 1.2-4.3 s to start. Details below.
- **Approximate progress bars.** The CLI reports no transfer progress, so
  the bar is a time-based estimate (cancelling still works).
- **Photos**: read-only, no albums, no upload to Photos
  ([#18](https://github.com/Aarklendoia/kio-protondrive/issues/18)).
  `/albums` doesn't work at all, and files in `/photos-shared-by-me`,
  `/photos-shared-with-me` and `/photos-trash` can be listed but not opened.
  Both are limits of the CLI itself (checked with CLI 0.9.0).
- **Proton Docs** can't be opened from Dolphin yet: open them from the web
  app ([#224](https://github.com/Aarklendoia/kio-protondrive/issues/224)).
- **Items in the trash** can't be copied out; restore them first (the CLI
  refuses both copying and downloading them).
- **Public link passwords** are passed to the CLI as a command-line argument
  (its only input method), so other local users could read one via
  `ps`/`/proc` during that short call.
- **Pinned files** are uploaded when edited locally; a change made to one
  elsewhere is picked up the next time it's opened through Dolphin, not
  continuously in the background.

<details>
<summary>Why there are no thumbnails</summary>

Dolphin/KIO's `PreviewJob` enforces a hard, non-configurable 2-second
timeout per file (`startTimer(2s)` in KDE Frameworks' `filepreviewjob.cpp`,
confirmed against the installed KF6 6.24.0 sources). Every `proton-drive`
CLI invocation carries 1.2-4.3s of its own fixed overhead regardless of file
size — confirmed live with `time proton-drive filesystem info` (1.2s) and
`time proton-drive photo download` (1.8-4.3s for files under 2.5 MB);
`user`+`sys` account for well under a second of that, so the rest is the
CLI's own Node.js/SDK session startup on every call, not network transfer.
No caching or worker-side optimization can close a gap this structural: the
CLI alone routinely takes longer than Dolphin allows for the entire preview
attempt, so every thumbnail request times out before the download can even
finish. Fixing this for real would mean keeping a long-lived,
already-authenticated `proton-drive` session warm across calls (a background
daemon brokering CLI access instead of spawning it fresh per operation) — a
much larger architecture change than this project's current on-demand design
(see [docs/DESIGN.md](docs/DESIGN.md)).

Separately, and independently of the timeout, HEIC photos (most iPhone
photos) have no working thumbnailer on Kubuntu 26.04 at all regardless of
protocol: the system's only HEIF/AVIF thumbnailer (`glycin-heif`) ships as an
external freedesktop.org `.thumbnailer` rather than a native KDE
`ThumbCreator` plugin, and isn't picked up by KIO's plugin loader in this
Frameworks version — a distro packaging gap, not something specific to
Proton Drive.

</details>

## Sign-in and credential storage

By default, the `proton-drive` CLI keeps its session in the desktop's Secret
Service (`libsecret`/GNOME Keyring/KWallet). That's fine for Dolphin, which
only runs inside an unlocked desktop session, but not for the background
sync daemon, which must authenticate with no one present to unlock a
keyring.

The packaged daemon service therefore sets
`PROTON_DRIVE_CREDENTIALS_STORE=unsafe_file` (supported by the CLI itself),
which keeps the session in `~/.local/share/proton-drive-cli/auth-session.json`
— readable only by you (`0600`), but **not encrypted at rest**, unlike the
keyring. The setup wizard lets you pick the store, including `pass`
(GPG-encrypted) instead.

<details>
<summary>Changing the store by hand, and keeping Dolphin in step</summary>

To use `pass` without the wizard, override the service:

```console
$ systemctl --user edit kio-protondrive-sync-daemon.service
```

```ini
[Service]
Environment=PROTON_DRIVE_CREDENTIALS_STORE=pass
```

(`pass` requires `pass` and a GPG key already set up, and `gpg-agent` able
to decrypt without an interactive prompt for the daemon to authenticate
unattended.)

On some setups (e.g. a Secret Service provider with no persistent on-disk
collection), the keyring store also silently fails to keep the session
across a keyring-daemon restart, forcing a fresh `proton-drive auth login`
every time — another reason the daemon doesn't use it.

The KIO worker runs inside Dolphin and never sees the daemon's systemd
`Environment=`: on its own, it would keep using the CLI's default store (the
keyring) or whatever your shell environment selects, so Dolphin and the
daemon could disagree about whether you're signed in. To avoid that, the
wizard's credential-storage page also writes
`~/.config/plasma-workspace/env/kio-protondrive.sh`, which exports
`PROTON_DRIVE_CREDENTIALS_STORE` for your whole Plasma session. Plasma only
reads that directory at login, so **log out and back in** after running the
wizard. If you change the store by editing the systemd unit directly (as
above), that script isn't updated: set `PROTON_DRIVE_CREDENTIALS_STORE` in
your session environment yourself, or re-run the wizard.

</details>

## How it works

```text
Dolphin  ──KIO protocol──▶  kio_protondrive (KF6::KIOCore plugin, C++)
                                  │
                                  │  cxx FFI bridge
                                  ▼
                          protondrive-core (Rust)  ◀──▶  local cache (SQLite)
                                  │
                                  │  subprocess + JSON (-j)
                                  ▼
                          proton-drive CLI  ──▶  Proton Drive

kio-protondrive-daemon (systemd --user): pinned-file sync, cache cleanup,
cache revalidation, CLI update checks
```

- **`core/`** (Rust): all the actual logic. It runs the official
  `proton-drive` CLI, parses its JSON output, maps KIO paths to Proton Drive
  paths and CLI errors to KIO errors, and owns the local caches. Unit tested
  against recorded JSON fixtures (see `core/src/cli.rs`), so `cargo test`
  needs no network access or Proton Drive session.
- **`worker/`**: a thin C++ shim (there's no Rust binding for
  `KIO::WorkerBase`) implementing the KIO protocol methods
  (`listDir`/`stat`/`mimetype`/`get`/`put`/`mkdir`/`del`/`rename`/`copy`)
  and the Dolphin plugins (context-menu actions, status badges, share
  dialog), calling into `core/` through a [`cxx`](https://cxx.rs) bridge.
- **`daemon/`** (Rust): the background service. It uploads local edits to
  pinned files, evicts unused cached files, re-checks folders Dolphin just
  showed from the cache and tells Dolphin to refresh them when they changed,
  sweeps the whole cache every 15 minutes, and checks for CLI updates.
- **`wizard/`** (Rust + QML): the setup wizard.
- Built with [Corrosion](https://github.com/corrosion-rs/corrosion), which
  drives the Rust build from CMake.

Files are fetched **on demand**, like `sftp://`, not synchronized to a local
folder in the background: only what you open or pin is stored locally. See
[docs/DESIGN.md](docs/DESIGN.md) for the design decisions behind the caches,
pinning, transfers and trash.

## Building from source

```bash
sudo apt-get install build-essential cmake extra-cmake-modules pkg-config \
  cargo rustc qt6-base-dev qt6-base-dev-tools qt6-l10n-tools libkf6kio-dev \
  libkf6coreaddons-dev libkf6widgetsaddons-dev libsqlite3-dev
cargo build --manifest-path core/Cargo.toml   # Rust core only, for quick iteration
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build
cmake --install build --prefix "$HOME/.local"  # test without root
```

Requires Rust 1.93 or newer. See [CONTRIBUTING.md](CONTRIBUTING.md) for the
full development workflow (including the dependency constraints imposed by
the packaged builds), and [docs/LAUNCHPAD.md](docs/LAUNCHPAD.md) for the
Debian/Launchpad release process.

## Contributing

Bug reports, ideas and pull requests are welcome on
[GitHub](https://github.com/Aarklendoia/kio-protondrive/issues). Open items
include albums and uploading to Photos
([#18](https://github.com/Aarklendoia/kio-protondrive/issues/18)), opening
Proton Docs ([#224](https://github.com/Aarklendoia/kio-protondrive/issues/224))
and refreshing open folders automatically
([#225](https://github.com/Aarklendoia/kio-protondrive/issues/225)). Start
with [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[GPL-3.0-or-later](LICENSE).

Proton and Proton Drive are trademarks of Proton AG. This project is not
affiliated with Proton AG.
