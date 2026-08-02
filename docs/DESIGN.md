# Design notes

## Why the KIO worker is on-demand, not a local mirror

`worker/` + `core/` implement `protondrive://` the same way KIO's own
`sftp://` or `kio-gdrive` work: every browse/open/upload is a direct,
synchronous round-trip to the `proton-drive` CLI, with nothing persisted
locally beyond the current process's lifetime. No background daemon, no
local database, no cache of what's "in sync."

That's a deliberate boundary, not a missing feature:

- **Correctness is simpler.** A stateless worker can't drift out of sync
  with reality — there's no local index that can go stale, get corrupted,
  or disagree with what Proton Drive actually has. Every call reflects the
  current server state.
- **Conflict resolution doesn't belong in a KIO worker.** `KIO::WorkerBase`
  has no protocol for surfacing "this file changed on both sides, which
  do you want?" — that's a background-sync concept, not a
  browse-and-open-a-file concept. Baking it into the worker would mean
  inventing non-standard behavior that breaks the moment another
  KIO-aware app (not just Dolphin) talks to `protondrive://`.
- **Scope stays bounded.** The worker's whole job is translating KIO calls
  to CLI calls (see `core/src/cli.rs`) and back. Anything stateful —
  watching a folder, deciding what changed, retrying failures — is a
  different lifecycle (a long-running background process) with different
  failure modes than a short-lived KIO worker process Dolphin spawns and
  kills per session.

That stateful, background half of the picture is the planned **sync
daemon** — see below.

## Sync daemon: pin/cache model

Tracked in [#30](https://github.com/Aarklendoia/kio-protondrive/issues/30),
**implemented**. Originally scoped as [#12](https://github.com/Aarklendoia/kio-protondrive/issues/12)'s
one-configured-folder, bi-directional mirror (see "Superseded #12 design"
below) — that model was abandoned before being finished because it doesn't
match how `protondrive:/` is actually used: on demand, browsed directly,
the same way `sftp://` is, with no separate "sync folder" most users would
have to think about maintaining. What people actually wanted was a way to
mark *specific* files/folders for guaranteed local availability, not a
second folder to keep mentally in sync with the first.

- **Everything stays on-demand by default.** `protondrive:/` browsing is
  unchanged from the stateless model described above — nothing is cached
  opportunistically.
- **Pinning is the only thing that persists a local copy.** A Dolphin
  ServiceMenu (`daemon/kio-protondrive-pin.desktop`, filtered to
  `protondrive://` via `X-KDE-Protocols`) adds "Garder en local" / "Supprimer
  la copie locale" to the right-click menu, each shelling out to
  `kio-protondrive-daemon pin|unpin <url>`. That's a one-shot client for the
  already-running daemon's own local control server (`daemon/src/control.rs`,
  same hand-rolled local-HTTP pattern as the wizard's), keeping the pin index
  single-writer.
- **Pin index: `core/src/cache.rs`.** A SQLite table (`remote_path ->
  local_path, local_mtime, local_size, last_synced_at`) — persistent, at
  `$XDG_DATA_HOME/kio-protondrive/cache-index.sqlite3`, since pin *state* is
  user intent, not disposable. The actual downloaded bytes live under
  `$XDG_CACHE_HOME/kio-protondrive/files/` (mirroring remote paths) —
  regenerable, safe to wipe (a pin just re-downloads next access).
- **Worker reads the pin index directly.** `get`/`stat` on a pinned path are
  served straight from the local copy — no CLI round-trip, no daemon
  involvement — via cxx bridge functions (`lookup_pin`/`pin_path`/
  `unpin_path`) that call into `core::cache` from C++. This is the "instant"
  case the whole feature exists for.
- **Change detection: `inotify`, scoped to the fixed cache root.** Same
  debounced-watch mechanism as the original design, just watching
  `Cache::default_root()` instead of a user-configured path — a local edit
  to a pinned file's cached copy triggers an upload of just that file, via
  `Cache::lookup_by_local_path` to recover which remote path it belongs to.
- **Direction: one-way local → Drive, for pinned files only.** Picking up a
  remote change made elsewhere (another device, the web app) to a pinned
  file happens the next time it's accessed through the worker (which
  re-downloads it), not via continuous background polling of Drive — chosen
  deliberately to keep the daemon's job close to "upload what changed
  locally," not a new poller against Drive's API.
- **No conflict resolution, no eviction.** Both existed only because the old
  design cached opportunistically and synced bi-directionally. Neither
  applies here: nothing is ever cached without the user explicitly pinning
  it, so there's nothing to age out ("cleanup" is just unpinning), and
  there's no continuous two-way sync to produce a same-file-changed-on-
  both-sides conflict in the first place.
- **Process model: unchanged from the original design** — `daemon/`, a
  `systemd --user` service depending on `core/` for all `proton-drive` CLI
  interaction, independent lifecycle from the per-session KIO worker
  process. It now also runs the pin control server described above.
- **Error/retry handling: unchanged in spirit.** A pinned file that fails to
  upload keeps its stale `last_synced_at` in the pin index and is retried
  automatically on the next `inotify` event or the startup reconciliation
  pass — no separate backoff/retry-queue mechanism.

### Superseded #12 design (not built)

The original plan for #12 was a single configured local folder mapped to a
single Drive folder, synced bi-directionally, using a local SQLite journal
to distinguish "changed locally" from "changed on both sides since last
sync" (the same approach Dropbox/Nextcloud/`rclone bisync` use), with
same-name conflicts resolved by renaming the local copy (e.g. `file
(conflict YYYY-MM-DD).ext`) rather than discarding either version. None of
that was implemented before the pivot to the pin/cache model above — kept
here only as a record of the design considered and abandoned, in case
bi-directional whole-folder sync is revisited later as a distinct feature
from pinning.
