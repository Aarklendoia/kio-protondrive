//! Read-only support for Proton Drive's Photos section (see #18).
//!
//! `/photos` isn't a real Drive path — `filesystem list`/`info` reject it
//! outright — Proton Drive CLI 0.7.0+ instead exposes photos through a
//! separate `photo` command family, addressed by `nodeUid`. This module
//! bridges that back into the worker's path-based world: it assigns each
//! photo a stable display name under a synthetic, flat `/photos/<name>`
//! namespace, deterministically derived from the full timeline so the same
//! name always resolves back to the same node.
//!
//! Unlike the rest of this crate (see docs/DESIGN.md's on-demand, stateless
//! design), the functions here are *not* cheap to call repeatedly — the
//! underlying `photo timeline -d` call has no pagination or per-item
//! lookup and can take well over a minute for a large library (see
//! [`crate::cli::photo_timeline`]) — so `crate::bridge` memoizes their
//! result in `crate::cache::Cache`'s on-disk `photo_timeline_cache` table
//! (shared across every process that opens it, not just this one) rather
//! than calling straight through on every `stat`/`get`.

use std::collections::HashMap;

use crate::cli::{self, CommandRunner, DriveError};
use crate::entry::NodeEntry;

/// A photo paired with the name it's addressed by under `/photos/`.
#[derive(Debug, Clone)]
pub struct Photo {
    pub display_name: String,
    pub node: NodeEntry,
}

/// Lists every photo in the account (fresh — no caching of its own, see
/// this module's doc comment) with [`disambiguate`] applied.
pub fn list_photos(runner: &dyn CommandRunner) -> Result<Vec<Photo>, DriveError> {
    Ok(disambiguate(cli::photo_timeline(runner)?))
}

/// Resolves a `/photos/<name>` display name back to its node — re-lists and
/// reapplies [`list_photos`]'s disambiguation. See this module's doc
/// comment: not cheap on its own, callers are expected to memoize.
pub fn find_photo(runner: &dyn CommandRunner, name: &str) -> Result<Photo, DriveError> {
    list_photos(runner)?
        .into_iter()
        .find(|photo| photo.display_name == name)
        .ok_or_else(|| DriveError::NotFound(format!("/photos/{name}")))
}

/// Assigns each node a unique `display_name`: the decrypted filename (or
/// the raw `uid` when undecryptable, matching [`NodeEntry::display_name`],
/// with any `^<hash>^` import-dedup prefix stripped — see
/// [`strip_import_hash_prefix`]), with a " (2)", " (3)", ... suffix added
/// after the first occurrence of a name. Proton Drive does allow multiple
/// photos with the same filename, and the CLI's own `photo download -c`
/// conflict-strategy handling doesn't disambiguate them either ("only one
/// of the images will be kept" per `photo download --help`) — this exists
/// so every photo still gets a distinct, clickable entry in Dolphin.
///
/// Deterministic given the same `nodes` (a stable, newest-first order from
/// the CLI, or from `crate::cache`'s on-disk cache of it) — split out from
/// [`list_photos`] so a cached node list can be re-disambiguated without
/// another CLI call; see `crate::bridge`'s `cached_photos`.
pub fn disambiguate(nodes: Vec<NodeEntry>) -> Vec<Photo> {
    let mut seen: HashMap<String, u32> = HashMap::new();
    nodes
        .into_iter()
        .map(|node| {
            let base = strip_import_hash_prefix(node.display_name()).to_string();
            let count = seen.entry(base.clone()).or_insert(0);
            *count += 1;
            let display_name = if *count == 1 {
                base
            } else {
                match base.rsplit_once('.') {
                    Some((stem, ext)) => format!("{stem} ({}).{ext}", *count),
                    None => format!("{base} ({})", *count),
                }
            };
            Photo { display_name, node }
        })
        .collect()
}

/// Proton Drive's own import pipeline disambiguates duplicate original
/// filenames server-side by prepending `^<hex-content-hash>^` to the name —
/// confirmed live against a real account (repeated saves of a generically-
/// named `pimgpsh_mobile_save_distr.jpg`, likely from Pinterest, ended up
/// as `^B497956E813EF883353483FC5B9269A0E87AC3037EA7C0796A^pimgpsh_mobile_\
/// save_distr.jpg`). The hash means nothing to a Dolphin user — stripped
/// here for display; any resulting collision is caught by [`disambiguate`]'s
/// own " (2)", " (3)", ... suffix same as it would be without this prefix
/// ever existing.
fn strip_import_hash_prefix(name: &str) -> &str {
    let Some(rest) = name.strip_prefix('^') else {
        return name;
    };
    let Some(caret_at) = rest.find('^') else {
        return name;
    };
    let (hash, after) = rest.split_at(caret_at);
    let looks_like_a_hash = hash.len() >= 8 && hash.bytes().all(|b| b.is_ascii_hexdigit());
    if looks_like_a_hash {
        &after[1..]
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::CommandOutput;
    use std::time::Duration;

    struct MockRunner {
        stdout: String,
    }

    impl CommandRunner for MockRunner {
        fn run(&self, _args: &[&str], _timeout: Duration) -> Result<CommandOutput, DriveError> {
            Ok(CommandOutput {
                stdout: self.stdout.clone(),
                stderr: String::new(),
                success: true,
            })
        }
    }

    #[test]
    fn strip_import_hash_prefix_removes_a_real_proton_dedup_prefix() {
        assert_eq!(
            strip_import_hash_prefix(
                "^B497956E813EF883353483FC5B9269A0E87AC3037EA7C0796A^pimgpsh_mobile_save_distr.jpg"
            ),
            "pimgpsh_mobile_save_distr.jpg"
        );
    }

    #[test]
    fn strip_import_hash_prefix_leaves_ordinary_names_alone() {
        assert_eq!(strip_import_hash_prefix("IMG_0001.HEIC"), "IMG_0001.HEIC");
        // Starts with '^' but the segment before the next '^' isn't hex —
        // not the pattern this is meant to strip, leave it as-is.
        assert_eq!(
            strip_import_hash_prefix("^not-a-hash^rest.jpg"),
            "^not-a-hash^rest.jpg"
        );
        // Only one '^' at all — no closing delimiter, leave it as-is.
        assert_eq!(
            strip_import_hash_prefix("^lonely-caret.jpg"),
            "^lonely-caret.jpg"
        );
    }

    fn node(uid: &str, name: &str) -> String {
        format!(
            r#"{{"uid":"{uid}","name":{{"ok":true,"value":"{name}"}},"type":"photo","mediaType":"image/jpeg","totalStorageSize":1234,"creationTime":"2026-01-01T00:00:00.000Z","modificationTime":"2026-01-01T00:00:00.000Z"}}"#
        )
    }

    #[test]
    fn list_photos_disambiguates_duplicate_names_with_a_numeric_suffix() {
        let stdout = format!(
            "[{},{},{}]",
            node("uid-1", "IMG_0001.HEIC"),
            node("uid-2", "IMG_0001.HEIC"),
            node("uid-3", "vacation.jpg")
        );
        let runner = MockRunner { stdout };
        let photos = list_photos(&runner).unwrap();
        let names: Vec<&str> = photos.iter().map(|p| p.display_name.as_str()).collect();
        assert_eq!(
            names,
            vec!["IMG_0001.HEIC", "IMG_0001 (2).HEIC", "vacation.jpg"]
        );
    }

    #[test]
    fn find_photo_resolves_a_disambiguated_name_back_to_the_right_uid() {
        let stdout = format!(
            "[{},{}]",
            node("uid-1", "IMG_0001.HEIC"),
            node("uid-2", "IMG_0001.HEIC")
        );
        let runner = MockRunner { stdout };
        let photo = find_photo(&runner, "IMG_0001 (2).HEIC").unwrap();
        assert_eq!(photo.node.uid, "uid-2");
    }

    #[test]
    fn find_photo_errors_when_no_photo_has_that_name() {
        let runner = MockRunner {
            stdout: "[]".to_string(),
        };
        let err = find_photo(&runner, "nope.jpg").unwrap_err();
        assert_eq!(err, DriveError::NotFound("/photos/nope.jpg".to_string()));
    }
}
