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

/// The web app's `/photos` filter tabs (see `crate::entry::PhotoDetails`'s
/// doc comment for where the underlying tag numbers come from). `LivePhotos`
/// matches *both* tag 3 and tag 4 — Proton's own web client combines
/// LivePhoto/MotionPhoto (iOS's and Android's equivalent formats) into one
/// "Live Photos" filter, confirmed in their `Tags.tsx`'s `PhotosTagsProps`
/// handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhotoCategory {
    Favorites,
    Screenshots,
    Videos,
    LivePhotos,
    Selfies,
    Portraits,
    Bursts,
    Panoramas,
    Raw,
}

impl PhotoCategory {
    pub const ALL: [PhotoCategory; 9] = [
        PhotoCategory::Favorites,
        PhotoCategory::Screenshots,
        PhotoCategory::Videos,
        PhotoCategory::LivePhotos,
        PhotoCategory::Selfies,
        PhotoCategory::Portraits,
        PhotoCategory::Bursts,
        PhotoCategory::Panoramas,
        PhotoCategory::Raw,
    ];

    /// The `/photos/<slug>` path segment this category is addressed by.
    pub fn slug(self) -> &'static str {
        match self {
            PhotoCategory::Favorites => "favorites",
            PhotoCategory::Screenshots => "screenshots",
            PhotoCategory::Videos => "videos",
            PhotoCategory::LivePhotos => "live-photos",
            PhotoCategory::Selfies => "selfies",
            PhotoCategory::Portraits => "portraits",
            PhotoCategory::Bursts => "bursts",
            PhotoCategory::Panoramas => "panoramas",
            PhotoCategory::Raw => "raw",
        }
    }

    /// The tag number(s) (see `crate::entry::PhotoDetails`) that put a photo
    /// in this category.
    pub fn tags(self) -> &'static [u8] {
        match self {
            PhotoCategory::Favorites => &[0],
            PhotoCategory::Screenshots => &[1],
            PhotoCategory::Videos => &[2],
            PhotoCategory::LivePhotos => &[3, 4],
            PhotoCategory::Selfies => &[5],
            PhotoCategory::Portraits => &[6],
            PhotoCategory::Bursts => &[7],
            PhotoCategory::Panoramas => &[8],
            PhotoCategory::Raw => &[9],
        }
    }

    pub fn from_slug(slug: &str) -> Option<PhotoCategory> {
        Self::ALL.into_iter().find(|c| c.slug() == slug)
    }
}

/// Filters an already-fetched, already-disambiguated photo list (e.g.
/// [`list_photos`]'s result, or `crate::bridge`'s cached equivalent) down to
/// `category`'s members. Split out from [`list_photos_by_category`] so
/// `crate::bridge`'s memoized `/photos` timeline can be filtered without a
/// second CLI round-trip. Filtering an already-disambiguated list (not
/// re-running disambiguation on a pre-filtered subset) matters: a photo's
/// `/photos/<name>` display name must be identical whether it's reached
/// through `/photos` directly or through a `/photos/<category>` filter —
/// re-disambiguating a smaller list could assign a same-named photo a
/// different " (2)" suffix depending on which other same-named photos
/// happened to also match the filter.
pub fn filter_by_category(photos: &[Photo], category: PhotoCategory) -> Vec<Photo> {
    photos
        .iter()
        .filter(|photo| {
            photo
                .node
                .photo
                .as_ref()
                .is_some_and(|p| p.tags.iter().any(|t| category.tags().contains(t)))
        })
        .cloned()
        .collect()
}

/// Lists every photo tagged with `category` — a fresh, non-cached call
/// (like [`list_photos`]) combining it with [`filter_by_category`].
pub fn list_photos_by_category(
    runner: &dyn CommandRunner,
    category: PhotoCategory,
) -> Result<Vec<Photo>, DriveError> {
    Ok(filter_by_category(&list_photos(runner)?, category))
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

    fn node_with_tags(uid: &str, name: &str, tags: &[u8]) -> String {
        let tags_json = tags.iter().map(u8::to_string).collect::<Vec<_>>().join(",");
        format!(
            r#"{{"uid":"{uid}","name":{{"ok":true,"value":"{name}"}},"type":"photo","mediaType":"image/jpeg","totalStorageSize":1234,"creationTime":"2026-01-01T00:00:00.000Z","modificationTime":"2026-01-01T00:00:00.000Z","photo":{{"tags":[{tags_json}]}}}}"#
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

    #[test]
    fn list_photos_by_category_filters_by_tag() {
        let stdout = format!(
            "[{},{},{}]",
            node_with_tags("uid-1", "video.mp4", &[2]),
            node_with_tags("uid-2", "screenshot.png", &[1]),
            node("uid-3", "untagged.jpg"),
        );
        let runner = MockRunner { stdout };
        let photos = list_photos_by_category(&runner, PhotoCategory::Videos).unwrap();
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].node.uid, "uid-1");
    }

    #[test]
    fn list_photos_by_category_matches_either_tag_for_live_photos() {
        let stdout = format!(
            "[{},{},{}]",
            node_with_tags("uid-1", "live.heic", &[3]),
            node_with_tags("uid-2", "motion.jpg", &[4]),
            node_with_tags("uid-3", "plain.heic", &[5]),
        );
        let runner = MockRunner { stdout };
        let photos = list_photos_by_category(&runner, PhotoCategory::LivePhotos).unwrap();
        let uids: Vec<&str> = photos.iter().map(|p| p.node.uid.as_str()).collect();
        assert_eq!(uids, vec!["uid-1", "uid-2"]);
    }

    #[test]
    fn list_photos_by_category_preserves_the_full_lists_disambiguation_suffix() {
        // Only the second IMG_0001 is tagged Favorites — filtering must not
        // re-disambiguate against just the filtered subset (which would
        // wrongly drop its " (2)" suffix back to a bare name), since that
        // would make the same photo resolve to two different `/photos/...`
        // names depending on whether it's reached flat or through a filter.
        let stdout = format!(
            "[{},{}]",
            node("uid-1", "IMG_0001.HEIC"),
            node_with_tags("uid-2", "IMG_0001.HEIC", &[0]),
        );
        let runner = MockRunner { stdout };
        let photos = list_photos_by_category(&runner, PhotoCategory::Favorites).unwrap();
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].display_name, "IMG_0001 (2).HEIC");
    }

    #[test]
    fn photo_category_slugs_round_trip() {
        for category in PhotoCategory::ALL {
            assert_eq!(PhotoCategory::from_slug(category.slug()), Some(category));
        }
        assert_eq!(PhotoCategory::from_slug("not-a-category"), None);
    }
}
