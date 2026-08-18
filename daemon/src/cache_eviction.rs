//! Periodic eviction sweep for `core::cache`'s opportunistic file cache
//! (issue #60) — every `get()`/`put()` in `worker/protondriveworker.cpp`
//! leaves a downloaded/uploaded file's bytes on disk afterward instead of
//! discarding them immediately, so a repeat open is instant. This module is
//! what eventually reclaims that space: it never touches `pins` (explicit,
//! permanent, `unpin()`-only cleanup) — only the separate `cached_files`
//! table populated by opportunistic caching.
//!
//! Purely local: no `proton-drive` CLI call, no network round trip, just
//! comparing each entry's `last_accessed_at` against the configured
//! retention window (`daemon/src/config.rs`'s `cache_retention_days`) and
//! deleting what's aged out (`Cache::evict_cached_file`, which removes both
//! the local file and its SQLite row). No D-Bus notification afterward,
//! unlike `fs_refresh`'s sweep for #8: a file going from "available
//! locally" back to "cloud-only" doesn't need an immediate icon repaint —
//! the overlay plugin's next natural query for that path already reflects
//! it, and there's no open Dolphin *listing* view actively displaying stale
//! content the way a stale directory listing would be.

use std::time::Duration;

use protondrive_core::cache::Cache;

pub fn evict_stale(cache: &Cache, retention: Duration) {
    let stale = match cache.stale_cached_files(retention) {
        Ok(paths) => paths,
        Err(err) => {
            log::warn!("cache eviction: could not list stale cached files: {err}");
            return;
        }
    };
    for path in stale {
        if let Err(err) = cache.evict_cached_file(&path) {
            log::warn!("cache eviction: failed to evict {path}: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn cache() -> (tempfile::TempDir, Cache) {
        let dir = tempfile::tempdir().unwrap();
        let cache =
            Cache::open(&dir.path().join("index.sqlite3"), &dir.path().join("files")).unwrap();
        (dir, cache)
    }

    #[test]
    fn evicts_everything_with_a_zero_retention_window() {
        let (_dir, cache) = cache();
        let target_dir = cache.target_dir_for("/my-files/a.txt").unwrap();
        let local_path = target_dir.join("a.txt");
        std::fs::write(&local_path, b"hello").unwrap();
        cache
            .store_cached_file("/my-files/a.txt", &local_path, "2026-01-01T00:00:00.000Z")
            .unwrap();

        evict_stale(&cache, Duration::ZERO);

        assert!(cache.cached_file("/my-files/a.txt").unwrap().is_none());
        assert!(!local_path.exists());
    }

    #[test]
    fn leaves_a_freshly_cached_file_alone_under_a_generous_retention_window() {
        let (_dir, cache) = cache();
        let target_dir = cache.target_dir_for("/my-files/a.txt").unwrap();
        let local_path = target_dir.join("a.txt");
        std::fs::write(&local_path, b"hello").unwrap();
        cache
            .store_cached_file("/my-files/a.txt", &local_path, "2026-01-01T00:00:00.000Z")
            .unwrap();

        evict_stale(&cache, Duration::from_secs(30 * 24 * 60 * 60));

        assert!(cache.cached_file("/my-files/a.txt").unwrap().is_some());
        assert!(local_path.exists());
    }

    /// Minimal double for `Cache::pin`'s two CLI calls (`filesystem info`
    /// then `filesystem download`) — same shape as `core::cache`'s own
    /// `DownloadingMockRunner`, trimmed to just what's needed here.
    struct PinningMockRunner;

    impl protondrive_core::cli::CommandRunner for PinningMockRunner {
        fn run(
            &self,
            args: &[&str],
            _timeout: Duration,
        ) -> Result<protondrive_core::cli::CommandOutput, protondrive_core::cli::DriveError>
        {
            use protondrive_core::cli::CommandOutput;
            if args[0..2] == ["filesystem", "info"] {
                return Ok(CommandOutput {
                    stdout: r#"{"uid":"uid-1","name":{"ok":true,"value":"a.txt"},"type":"file","isShared":false,"creationTime":"2026-01-01T00:00:00.000Z","modificationTime":"2026-01-01T00:00:00.000Z"}"#.to_string(),
                    stderr: String::new(),
                    success: true,
                });
            }
            if args[0..2] == ["filesystem", "download"] {
                let remote_path = args[args.len() - 2];
                let local_folder = args[args.len() - 1];
                let file_name = Path::new(remote_path).file_name().unwrap();
                std::fs::write(Path::new(local_folder).join(file_name), b"hello").unwrap();
                return Ok(CommandOutput {
                    stdout: r#"{"transferredItems":1,"transferredBytes":5,"skippedItems":0,"failedItems":0,"failures":[]}"#.to_string(),
                    stderr: String::new(),
                    success: true,
                });
            }
            panic!("unexpected CLI call: {args:?}");
        }
    }

    #[test]
    fn never_evicts_a_pinned_file() {
        let (_dir, cache) = cache();
        let local_path = cache
            .pin(&PinningMockRunner, "/my-files/a.txt", false)
            .unwrap();

        evict_stale(&cache, Duration::ZERO);

        assert!(cache.lookup("/my-files/a.txt").unwrap().is_some());
        assert!(local_path.exists());
    }
}
