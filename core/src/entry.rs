//! Types mirroring the JSON shapes returned by `proton-drive -j ...`.
//!
//! Schema observed by running the real CLI (`filesystem list/info/
//! create-folder -j`, `filesystem upload/download -j`, `filesystem trash -j`)
//! against a live Proton Drive account. Fields not needed by the worker are
//! intentionally omitted rather than mapped.

use serde::{Deserialize, Serialize};

/// Proton Drive encrypts node names; `ok` is false when the name could not be
/// decrypted (e.g. a key/permission issue), in which case `value` is absent
/// and the caller falls back to displaying the node's `uid`.
///
/// `Serialize` (unusual for this module — everything else here only ever
/// flows one way, parsed from the CLI's JSON) is needed so `crate::cache`
/// can round-trip a `Vec<NodeEntry>` through its on-disk `/photos` timeline
/// cache (see `crate::photos`'s doc comment for why that cache exists).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecryptedField {
    pub ok: bool,
    #[serde(default)]
    pub value: Option<String>,
}

/// A file or folder node, as returned by `filesystem list`, `filesystem info`
/// and `filesystem create-folder`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeEntry {
    pub uid: String,
    pub name: DecryptedField,
    #[serde(rename = "type")]
    pub node_type: String,
    #[serde(default)]
    pub media_type: Option<String>,
    #[serde(default)]
    pub total_storage_size: Option<u64>,
    pub creation_time: String,
    pub modification_time: String,
    #[serde(default)]
    pub is_shared: bool,
    /// Whether the node currently has an active public link — confirmed
    /// live on a `filesystem list` node. `sharing status` also carries this
    /// (see `SharingStatus::url_access`), so `crate::sharing::status` is the
    /// richer read-only source when the caller already needs a status call
    /// anyway (`ShareDialog`); this field is what backs the cheap
    /// `worker/overlayplugin.cpp` "shared" badge instead, via
    /// `crate::bridge::lookup_shared`'s `fs_stat_cache` read.
    #[serde(default)]
    pub is_shared_by_url: bool,
    /// Only present on nodes from `photo timeline -d` (absent, and left
    /// `None`, for `filesystem list`/`info` nodes) — see
    /// `crate::photos::PhotoCategory` for what `tags` means.
    #[serde(default)]
    pub photo: Option<PhotoDetails>,
}

/// The `photo` sub-object `photo timeline -d` nests on each node —
/// confirmed live against a real account and cross-checked against
/// Proton's own web client source (`PhotoTag` in
/// `packages/shared/lib/interfaces/drive/file.ts`,
/// `github.com/ProtonMail/WebClients`): `tags` is a list of small integer
/// codes (0=Favorite, 1=Screenshot, 2=Video, 3=LivePhoto, 4=MotionPhoto,
/// 5=Selfie, 6=Portrait, 7=Burst, 8=Panorama, 9=Raw), server-computed, a
/// single photo can carry several at once.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PhotoDetails {
    #[serde(default)]
    pub tags: Vec<u8>,
}

impl NodeEntry {
    pub fn is_folder(&self) -> bool {
        self.node_type == "folder" || self.node_type == "album"
    }

    /// Display name: the decrypted name if available, otherwise the raw uid
    /// (better to show something than to fail the whole directory listing).
    pub fn display_name(&self) -> &str {
        self.name.value.as_deref().unwrap_or(&self.uid)
    }
}

/// The root path (`/`) doesn't list real nodes: it lists Proton Drive's
/// virtual top-level sections (`/my-files`, `/devices`, `/trash`, ...), each
/// represented as a bare `{"path": "..."}` object instead of a full node.
#[derive(Debug, Clone, Deserialize)]
pub struct VirtualSection {
    pub path: String,
}

impl VirtualSection {
    /// The last path segment, used as the folder's display name.
    pub fn display_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

/// `filesystem list` returns either virtual sections (when listing `/`) or
/// real nodes (when listing anything under a section) — never a mix, but the
/// two shapes must be told apart at parse time since neither has a
/// discriminant field of its own.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ListItem {
    Section(VirtualSection),
    Node(NodeEntry),
}

/// Result of `filesystem upload` / `filesystem download`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferSummary {
    pub transferred_items: u64,
    pub transferred_bytes: u64,
    pub skipped_items: u64,
    pub failed_items: u64,
}

/// One entry of the array returned by `filesystem trash`.
#[derive(Debug, Clone, Deserialize)]
pub struct TrashOutcome {
    pub uid: String,
    pub ok: bool,
}

/// Response of `sharing status -j path` — confirmed live, including a real
/// pending `nonProtonInvitations` entry (whose shape `protonInvitations` is
/// assumed to mirror, same underlying invitation mechanism). Also doubles
/// as `sharing set-url -j path`'s response shape: confirmed live that
/// set-url returns this same whole-status object with `urlAccess`
/// populated, not a flat link object.
///
/// `Default` backs `crate::cli::sharing_status`'s workaround for a separate
/// CLI bug: confirmed live that `sharing status -j` on a node that has
/// never been shared at all prints the literal text `undefined` (a
/// classic `JSON.stringify(undefined)` result) instead of an empty-but-
/// valid status object — this is that empty status.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharingStatus {
    #[serde(default)]
    pub proton_invitations: Vec<ProtonInvitation>,
    #[serde(default)]
    pub non_proton_invitations: Vec<NonProtonInvitation>,
    #[serde(default)]
    pub members: Vec<ShareMember>,
    #[serde(default)]
    pub editors_can_share: bool,
    #[serde(default)]
    pub url_access: Option<PublicLink>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareMember {
    pub invitee_email: String,
    pub role: String,
}

/// `state` is confirmed live as `"pending"` on every invitation observed so
/// far — captured (rather than assumed) so a future state this project
/// hasn't seen yet (accepted-but-not-yet-a-member? declined-but-not-yet-
/// removed?) doesn't get mislabeled "— pending" by `crate::sharing::status`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtonInvitation {
    pub invitee_email: String,
    pub role: String,
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NonProtonInvitation {
    pub invitee_email: String,
    pub role: String,
    #[serde(default)]
    pub state: Option<String>,
}

/// A node's public link, nested under `SharingStatus.url_access` —
/// confirmed live, including `expirationTime` when `--expiration` is set.
/// `number_of_initialized_downloads` is also confirmed live (present, at
/// 0, on a freshly created link) — how many times the link has been used,
/// worth surfacing next to the URL itself.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicLink {
    pub url: String,
    pub role: String,
    #[serde(default)]
    pub expiration_time: Option<String>,
    #[serde(default)]
    pub number_of_initialized_downloads: u64,
}
