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
    /// live on a `filesystem list` node. `sharing status`'s own response
    /// doesn't carry the link (see `SharingStatus`'s doc comment), so this
    /// is the only currently-known way to tell "has a link" apart from
    /// "doesn't" without calling `sharing set-url` (which creates/updates
    /// one, not a safe read-only check).
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

/// Response of `sharing status -j path` — confirmed live against a real
/// shared file. `protonInvitations`/`nonProtonInvitations` were empty on
/// every node tested so far, so their per-item shape below is this
/// project's best guess (mirroring `members`'s own shape, the only one
/// with confirmed fields) rather than something observed directly — to be
/// corrected once a real pending invitation can be inspected.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharingStatus {
    #[serde(default)]
    pub proton_invitations: Vec<ProtonInvitation>,
    #[serde(default)]
    pub non_proton_invitations: Vec<NonProtonInvitation>,
    #[serde(default)]
    pub members: Vec<ShareMember>,
    pub editors_can_share: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareMember {
    pub invitee_email: String,
    pub role: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtonInvitation {
    pub invitee_email: String,
    pub role: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NonProtonInvitation {
    pub invitee_email: String,
    pub role: String,
}

/// Response of `sharing set-url -j path` — shape not yet confirmed live
/// (see this module's `SharingStatus` doc comment and the plan's
/// Verification section); `url` is this project's best guess at the field
/// name for the created public link's address.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicLink {
    pub url: String,
    pub role: String,
    #[serde(default)]
    pub expiration_time: Option<String>,
}
