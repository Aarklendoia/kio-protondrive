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

use notify::event::{ModifyKind, RenameMode};
use notify::{Event, EventKind, RecursiveMode, Watcher as _};
use notify_debouncer_full::{new_debouncer, DebounceEventResult};

use crate::error::DaemonError;

/// Coalesces rapid successive writes to the same file (e.g. an editor's
/// write-then-rename save pattern) into a single upload instead of one per
/// intermediate write.
const DEBOUNCE_WINDOW: Duration = Duration::from_secs(2);

/// A local filesystem change worth acting on, already classified from the
/// raw debounced `notify` event — see [`classify`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// A file was created, or its content changed.
    Changed(PathBuf),
    /// A file was renamed or moved within the watched tree.
    Renamed { from: PathBuf, to: PathBuf },
}

/// Classifies one raw debounced event into zero or more [`WatchEvent`]s.
/// Pulled out of the debouncer callback so it's unit-testable with synthetic
/// `notify::Event` values, without needing a real filesystem/inotify.
///
/// `notify-debouncer-full` already stitches inotify's separate rename-from/
/// rename-to events into a single `Modify(Name(RenameMode::Both))` event
/// with `paths: [old, new]` before this ever runs — no manual correlation
/// needed here (confirmed from the crate's own source).
///
/// Deletions (`EventKind::Remove(_)`) and a rename's uncorrelated "from"
/// half (`RenameMode::From` — the file left the watched tree, e.g. moved
/// out or renamed away before its "to" half arrived) are deliberately
/// ignored: local-delete propagation to Drive isn't implemented yet (see
/// docs/DESIGN.md's phased scope) — a `From` with no matching `To` means
/// there's nothing left locally to act on. A rename's uncorrelated "to"
/// half (`RenameMode::To` — a file appeared from outside the watched tree,
/// or its "from" half was already delivered in an earlier debounce window)
/// is treated as [`WatchEvent::Changed`]: safe, since it just means
/// uploading it fresh, same as a plain create.
fn classify(event: &Event) -> Vec<WatchEvent> {
    match &event.kind {
        EventKind::Create(_) => event
            .paths
            .iter()
            .cloned()
            .map(WatchEvent::Changed)
            .collect(),
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
            if let [from, to] = event.paths.as_slice() {
                vec![WatchEvent::Renamed {
                    from: from.clone(),
                    to: to.clone(),
                }]
            } else {
                // The debouncer is expected to always pair these up — this
                // is just a defensive fallback in case that guarantee ever
                // changes, so a rename still uploads rather than vanishing.
                log::warn!(
                    "rename event had {} paths, expected 2 ({:?}) — treating each as a plain change",
                    event.paths.len(),
                    event.paths
                );
                event
                    .paths
                    .iter()
                    .cloned()
                    .map(WatchEvent::Changed)
                    .collect()
            }
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => event
            .paths
            .iter()
            .cloned()
            .map(WatchEvent::Changed)
            .collect(),
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => Vec::new(),
        EventKind::Modify(_) => event
            .paths
            .iter()
            .cloned()
            .map(WatchEvent::Changed)
            .collect(),
        _ => Vec::new(),
    }
}

/// Watches `local_path` recursively. Returns a handle that must be kept
/// alive for as long as watching should continue (dropping it stops the
/// watch), and a channel of batches of classified changes — see
/// [`classify`] for what's included/excluded.
pub fn watch(
    local_path: &Path,
) -> Result<(impl Send + 'static, mpsc::Receiver<Vec<WatchEvent>>), DaemonError> {
    let (tx, rx) = mpsc::channel();

    let mut debouncer =
        new_debouncer(
            DEBOUNCE_WINDOW,
            None,
            move |result: DebounceEventResult| match result {
                Ok(events) => {
                    let changes: Vec<WatchEvent> =
                        events.iter().flat_map(|event| classify(event)).collect();
                    if !changes.is_empty() {
                        let _ = tx.send(changes);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: EventKind, paths: Vec<PathBuf>) -> Event {
        Event {
            kind,
            paths,
            attrs: Default::default(),
        }
    }

    #[test]
    fn classify_treats_create_as_changed() {
        let path = PathBuf::from("/watched/new.txt");
        let result = classify(&event(
            EventKind::Create(notify::event::CreateKind::File),
            vec![path.clone()],
        ));
        assert_eq!(result, vec![WatchEvent::Changed(path)]);
    }

    #[test]
    fn classify_treats_a_plain_content_modify_as_changed() {
        let path = PathBuf::from("/watched/existing.txt");
        let result = classify(&event(
            EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            vec![path.clone()],
        ));
        assert_eq!(result, vec![WatchEvent::Changed(path)]);
    }

    #[test]
    fn classify_treats_a_correlated_rename_as_renamed() {
        let from = PathBuf::from("/watched/old.txt");
        let to = PathBuf::from("/watched/new.txt");
        let result = classify(&event(
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            vec![from.clone(), to.clone()],
        ));
        assert_eq!(result, vec![WatchEvent::Renamed { from, to }]);
    }

    #[test]
    fn classify_ignores_an_uncorrelated_rename_from() {
        let result = classify(&event(
            EventKind::Modify(ModifyKind::Name(RenameMode::From)),
            vec![PathBuf::from("/watched/gone.txt")],
        ));
        assert!(result.is_empty());
    }

    #[test]
    fn classify_treats_an_uncorrelated_rename_to_as_changed() {
        let path = PathBuf::from("/watched/appeared.txt");
        let result = classify(&event(
            EventKind::Modify(ModifyKind::Name(RenameMode::To)),
            vec![path.clone()],
        ));
        assert_eq!(result, vec![WatchEvent::Changed(path)]);
    }

    #[test]
    fn classify_ignores_removes() {
        let result = classify(&event(
            EventKind::Remove(notify::event::RemoveKind::File),
            vec![PathBuf::from("/watched/deleted.txt")],
        ));
        assert!(result.is_empty());
    }

    #[test]
    fn classify_falls_back_to_changed_when_a_rename_has_the_wrong_number_of_paths() {
        let path = PathBuf::from("/watched/odd.txt");
        let result = classify(&event(
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            vec![path.clone()],
        ));
        assert_eq!(result, vec![WatchEvent::Changed(path)]);
    }
}
