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

## Filesystem listing/stat cache

Tracked in [#8](https://github.com/Aarklendoia/kio-protondrive/issues/8),
**implemented** (the listing-cache half of it — the thumbnails half of #8 is
separately documented as blocked, see the README's Scope section). A
deliberate, larger deviation from this doc's opening "on-demand, stateless"
philosophy than either the pin cache or the `/photos` timeline cache above —
worth spelling out explicitly, same as those two were.

- **What's cached.** Every `stat()`/`list_dir()` result for a real Drive
  path (not the virtual root `/`, whose fixed sections are cheap and
  low-value to cache) — `core/src/cache.rs`'s `fs_stat_cache` (`path ->
  NodeEntry`) and `fs_listing_cache` (`parent_path -> [NodeEntry]`) tables,
  same on-disk SQLite file as the pin index and `/photos` cache.
- **No TTL — permanent until invalidated or swept.** Unlike the `/photos`
  cache's 5-minute freshness cutoff, a hit here is served however old it is.
  Two things keep it from going stale forever:
  - This app's own writes invalidate exactly what they touch
    (`core/src/bridge.rs`'s `make_dir`/`upload_from`/`trash`/`rename_or_move`
    call `Cache::invalidate_stat`/`invalidate_listing` after a successful
    CLI call) — a self-caused change is reflected on the very next visit.
  - The sync daemon sweeps the whole cache periodically
    (`daemon/src/fs_refresh.rs`, `FS_CACHE_REFRESH_INTERVAL` = 15 minutes in
    `daemon/src/main.rs`), re-fetching every cached path sequentially (each
    CLI call already costs ~1-4s — see the README's thumbnail-limitation
    paragraph — so this is deliberately not parallelized) and firing
    `org.kde.KDirNotify.FilesChanged` for whatever it refreshed, so an open
    Dolphin window re-renders instead of silently drifting from what's
    actually on Drive.
- **The accepted tradeoff.** A change made to a cached path from *outside*
  this app (the web UI, another device) can lag behind by up to the sweep
  interval — this is the real cost being traded for instant repeat browsing
  and an instantly-labeled breadcrumb, and the reason this needed its own
  writeup rather than just reusing the `/photos` cache's TTL pattern.
- **Why no background thread from the worker itself.** A more "reactive"
  design would re-verify a cache hit in the background the moment it's
  served, so a folder actively being browsed stays maximally fresh. Rejected
  deliberately: a `protondrive://` KIO worker process is short-lived and
  poolable (`maxInstances` in `worker/protondrive.json`) — nothing guarantees
  it outlives a thread it spawned, so "fire a background refresh from the
  worker" is a best-effort measure that can silently vanish. All "keep this
  fresh" work belongs to the daemon instead, which is already long-running.

## Cancellable transfers and approximate progress

Tracked in [#9](https://github.com/Aarklendoia/kio-protondrive/issues/9).
`get()`/`put()` used to shell out to the `proton-drive` CLI via a single
blocking call, only returning once the whole transfer finished — no way to
cancel, and `KIO::WorkerBase::totalSize()`/`processedSize()` were never
called during the actual transfer, only for the local temp-file copy phase.

- **Cancellable, via `crate::transfer::TransferHandle`.** `get()`/`put()` now
  start the CLI subprocess through `core/src/transfer.rs` (not
  `CommandRunner` — that trait is still used unchanged by every other call,
  including the daemon's own fire-and-forget pinned-file sync, which has no
  interactive cancel button) and poll it in a loop, checking
  `wasKilled()` every ~200ms and killing the subprocess (its whole process
  group — see below) if the user cancels. `put()`'s local-write phase and
  `get()`'s final local-copy phase (`streamLocalFile()`) check `wasKilled()`
  too, so cancelling is responsive at every stage, not just during the
  network transfer.
- **Whole process group, not just the direct pid.** Confirmed live that
  `sh -c "sleep 30"` (used in `transfer.rs`'s own tests) forks a real child
  rather than exec-replacing itself — killing only the parent's pid leaves
  that child running as an orphan. Every spawned process gets its own
  process group (`process_group(0)`), and cancellation/timeout/drop all
  signal the negative pid (the whole group).
- **No real byte-accurate progress — deliberately.** The `proton-drive` CLI
  has no stable progress API: no `--progress` flag, no incremental JSON. Its
  `-v`/`--verbose` flag does print live debug lines with progress-shaped
  content (`block N: Uploaded`, JSON `bytesProcessed` metrics) — but
  confirmed live, on an error case, that verbose mode also moves the actual
  error detail onto stdout and reduces stderr to an unhelpful separator line
  (the exact symptom tracked in [#38](https://github.com/Aarklendoia/kio-protondrive/issues/38)).
  Parsing that undocumented output would mean also reverse-engineering error
  classification from stdout instead of the current stderr-based
  `ensure_success()`, for a progress bar with zero stability guarantee across
  CLI versions. Rejected as not worth it.
- **Progress is instead a rough, time-based estimate.** `transfer.rs` keeps a
  process-lifetime running average (bytes/sec) per direction (upload,
  download), seeded with a conservative default until this worker process
  completes its first real transfer. `processedSize()` is fed
  `elapsed × average_speed`, capped at 95% of the known total until the
  transfer is genuinely done (then snapped to the real total) — a
  believable-looking bar, not a real measurement. Resets every time a new
  worker process is spawned (`maxInstances` pooling); no attempt to persist
  it, since it was never meant to be precise.
- **`stdout`/`stderr` capture is unchanged.** Still buffered via
  `wait_with_output()` (never read line-by-line), so `-v` is never passed and
  `cli::ensure_success()`'s error classification is completely unaffected.
- **`getPhoto()`/`download_photo()` (the `/photos` preview path) is
  unchanged** — still the old blocking call. Photo downloads are small and
  already bounded by `PreviewJob`'s 2s timeout (see the thumbnails known
  limitation), so cancellation matters far less there.

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
