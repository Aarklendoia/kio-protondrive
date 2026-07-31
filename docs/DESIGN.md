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

## Sync daemon

Tracked in [#12](https://github.com/Aarklendoia/kio-protondrive/issues/12).
Being built in phases; **Phase 1** (`daemon/`, package
`kio-protondrive-sync-daemon`) implements one-way local → Drive upload with
`inotify` change detection and a SQLite journal used for upload idempotency
only. Everything else below — bi-directional sync, conflict resolution,
Drive → local download — is still design-only, not yet implemented. Design
decisions made so far:

- **Direction: bi-directional.** Not just local → Drive; changes on either
  side propagate to the other. This is what makes persistent local state
  (below) a hard requirement rather than a nice-to-have — see the next
  point.
- **Change detection: `inotify`.** Real-time local watch rather than
  polling. Needs to handle common editor save patterns (write-to-temp +
  rename) and debounce rapid successive writes to the same file.
- **Scope: one configured folder.** A single local path mapped to a single
  Proton Drive path, not an arbitrary multi-folder setup. Keeps the first
  version's state model and UX simple; multi-folder can be a later
  extension if there's demand.
- **State: persistent, local (SQLite).** This follows directly from
  choosing bi-directional sync. Without knowing what the *last
  successfully synced* state of a file was, there's no way to distinguish
  "changed locally only" (normal upload) from "changed on both sides since
  last sync" (real conflict) — timestamp comparison alone is fragile
  (clock skew, mtime granularity). This is why Dropbox, Nextcloud's
  desktop client, Google Drive, and `rclone bisync` all keep a local
  sync journal: for each file, local path, local mtime/size, remote path,
  and the remote revision/hash as of the last successful sync. Same
  pattern here.
- **Conflict handling: rename and keep both.** When a file changed on both
  sides since the last sync, keep the remote version under the original
  name and upload the local version under a renamed copy (e.g. `file
  (conflict YYYY-MM-DD).ext`, matching the Dropbox/Nextcloud convention).
  No data is ever silently discarded; the user reconciles the duplicate
  afterward if needed.
- **Process model: a new `daemon/` Rust crate**, depending on `core/` for
  all `proton-drive` CLI interaction (same `CommandRunner` abstraction
  `core/src/cli.rs` already uses for testability) — not duplicated logic.
  Runs as a `systemd --user` service, started with the desktop session.
  Separate binary from the KIO worker plugin; they share `core/` as a
  library dependency but have independent lifecycles (the worker is
  spawned per-session by `kioworker`, the daemon runs continuously).
- **Error/retry handling: natural retry on the next cycle.** Because sync
  state is already persisted, a file that fails to upload/download simply
  stays marked "not yet synced" in the local database and gets retried
  automatically on the next `inotify` event or reconciliation pass — no
  separate backoff/retry-queue mechanism needed on top of the sync
  journal that already exists.

Bi-directional sync, conflict handling, and Drive → local download are not
implemented yet. See #12 for the tracking issue.
