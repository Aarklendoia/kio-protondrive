//! inotify-based change detection via `notify` + `notify-debouncer-full`.
//!
//! `notify` alone delivers raw, un-coalesced events — a typical editor save
//! is write-to-temp-file + rename-over-target, which without debouncing
//! shows up as several separate raw events. `notify-debouncer-full` sits on
//! top and coalesces rapid events per debounce window, and treats renames as
//! renames rather than remove+create pairs.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{EventKind, RecursiveMode, Watcher as _};
use notify_debouncer_full::{new_debouncer, DebounceEventResult};

use crate::error::DaemonError;

/// Coalesces rapid successive writes to the same file (e.g. an editor's
/// write-then-rename save pattern) into a single upload instead of one per
/// intermediate write.
const DEBOUNCE_WINDOW: Duration = Duration::from_secs(2);

/// Watches `local_path` recursively. Returns a handle that must be kept
/// alive for as long as watching should continue (dropping it stops the
/// watch), and a channel of batches of changed file paths.
///
/// Phase 1 only reacts to creates/modifies — deletions are filtered out
/// here, deliberately: local-delete propagation to Drive isn't implemented
/// yet (see docs/DESIGN.md's phased scope).
pub fn watch(
    local_path: &Path,
) -> Result<(impl Send + 'static, mpsc::Receiver<Vec<PathBuf>>), DaemonError> {
    let (tx, rx) = mpsc::channel();

    let mut debouncer =
        new_debouncer(
            DEBOUNCE_WINDOW,
            None,
            move |result: DebounceEventResult| match result {
                Ok(events) => {
                    let paths: Vec<PathBuf> = events
                        .iter()
                        .filter(|event| {
                            matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_))
                        })
                        .flat_map(|event| event.paths.clone())
                        .collect();
                    if !paths.is_empty() {
                        let _ = tx.send(paths);
                    }
                }
                Err(errors) => {
                    for error in errors {
                        log::warn!("filesystem watch error: {error}");
                    }
                }
            },
        )
        .map_err(|source| DaemonError::Watch {
            path: local_path.to_path_buf(),
            source,
        })?;

    debouncer
        .watcher()
        .watch(local_path, RecursiveMode::Recursive)
        .map_err(|source| DaemonError::Watch {
            path: local_path.to_path_buf(),
            source,
        })?;

    Ok((debouncer, rx))
}
