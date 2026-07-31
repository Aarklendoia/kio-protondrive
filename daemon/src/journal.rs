//! Local SQLite journal, used purely for upload idempotency in Phase 1: it
//! remembers the local mtime/size we last successfully uploaded for each
//! file, so an unchanged file isn't re-uploaded on every reconcile pass or
//! debounced fs event. No remote revision tracking, no conflict detection —
//! that's Phase 2+ (bi-directional sync) territory, see docs/DESIGN.md.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::DaemonError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalRecord {
    pub local_mtime: i64,
    pub local_size: i64,
    pub last_synced_at: i64,
}

pub struct Journal {
    conn: Connection,
}

impl Journal {
    pub fn open(db_path: &Path) -> Result<Self, DaemonError> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS files (
                local_path TEXT PRIMARY KEY,
                local_mtime INTEGER NOT NULL,
                local_size INTEGER NOT NULL,
                last_synced_at INTEGER NOT NULL
            )",
            [],
        )?;
        Ok(Self { conn })
    }

    pub fn get(&self, local_path: &Path) -> Result<Option<JournalRecord>, DaemonError> {
        let key = local_path.to_string_lossy();
        let record = self
            .conn
            .query_row(
                "SELECT local_mtime, local_size, last_synced_at FROM files WHERE local_path = ?1",
                params![key],
                |row| {
                    Ok(JournalRecord {
                        local_mtime: row.get(0)?,
                        local_size: row.get(1)?,
                        last_synced_at: row.get(2)?,
                    })
                },
            )
            .optional()?;
        Ok(record)
    }

    /// Records `local_path` as successfully synced as of now, with the given
    /// mtime/size — call only after a successful upload.
    pub fn mark_synced(
        &self,
        local_path: &Path,
        local_mtime: i64,
        local_size: i64,
    ) -> Result<(), DaemonError> {
        let key = local_path.to_string_lossy();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.conn.execute(
            "INSERT INTO files (local_path, local_mtime, local_size, last_synced_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(local_path) DO UPDATE SET
                local_mtime = excluded.local_mtime,
                local_size = excluded.local_size,
                last_synced_at = excluded.last_synced_at",
            params![key, local_mtime, local_size, now],
        )?;
        Ok(())
    }

    /// Whether `local_path` needs uploading, given its current mtime/size —
    /// true when we have no record, or the file changed since we last synced
    /// it.
    pub fn needs_upload(
        &self,
        local_path: &Path,
        local_mtime: i64,
        local_size: i64,
    ) -> Result<bool, DaemonError> {
        Ok(match self.get(local_path)? {
            Some(record) => record.local_mtime != local_mtime || record.local_size != local_size,
            None => true,
        })
    }

    /// `~/.local/share/kio-protondrive/daemon.sqlite3`.
    pub fn default_path() -> PathBuf {
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let home = std::env::var_os("HOME").expect("HOME must be set");
                PathBuf::from(home).join(".local").join("share")
            });
        data_home.join("kio-protondrive").join("daemon.sqlite3")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal() -> (tempfile::TempDir, Journal) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("journal.sqlite3");
        let journal = Journal::open(&db_path).unwrap();
        (dir, journal)
    }

    #[test]
    fn needs_upload_is_true_for_an_unknown_file() {
        let (_dir, journal) = journal();
        assert!(journal
            .needs_upload(Path::new("/local/report.pdf"), 100, 200)
            .unwrap());
    }

    #[test]
    fn needs_upload_is_false_after_marking_synced_with_the_same_mtime_and_size() {
        let (_dir, journal) = journal();
        let path = Path::new("/local/report.pdf");
        journal.mark_synced(path, 100, 200).unwrap();
        assert!(!journal.needs_upload(path, 100, 200).unwrap());
    }

    #[test]
    fn needs_upload_is_true_again_after_the_file_changes() {
        let (_dir, journal) = journal();
        let path = Path::new("/local/report.pdf");
        journal.mark_synced(path, 100, 200).unwrap();
        assert!(journal.needs_upload(path, 101, 200).unwrap());
        assert!(journal.needs_upload(path, 100, 201).unwrap());
    }

    #[test]
    fn mark_synced_overwrites_the_previous_record() {
        let (_dir, journal) = journal();
        let path = Path::new("/local/report.pdf");
        journal.mark_synced(path, 100, 200).unwrap();
        journal.mark_synced(path, 150, 250).unwrap();
        let record = journal.get(path).unwrap().unwrap();
        assert_eq!(record.local_mtime, 150);
        assert_eq!(record.local_size, 250);
    }
}
