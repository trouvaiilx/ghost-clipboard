//! SQLite repository for clipboard history.
//!
//! Handles SQLite connection initialization, custom case folding functions,
//! migrations, indexing, and high-performance querying without blocking.

use rusqlite::{Connection, OpenFlags, Row, params};
use std::path::Path;

use crate::domain::entry::{ContentKind, EntryId, EntrySummary, NewEntry, preview_of};
use crate::domain::settings::{DedupMode, RetentionPolicy};
use crate::storage::Result;
use crate::storage::migrations::migrate;

/// Query parameters for listing history.
#[derive(Debug, Clone, Default)]
pub struct HistoryQuery {
    pub search_term: Option<String>,
    pub limit: usize,
    pub offset: usize,
}

/// The result of attempting to record an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordOutcome {
    Inserted(EntryId),
    Deduplicated { id: EntryId, moved_to_top: bool },
}

/// Statistics reported when opening the repository.
#[derive(Debug, Clone)]
pub struct OpenReport {
    pub total_entries: usize,
    pub pinned_entries: usize,
    pub schema_version: i64,
}

pub struct Repository {
    conn: Connection,
}

impl Repository {
    /// Open or create a database file at the given path.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Self::init(conn)
    }

    /// Open an in-memory database for testing.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        // WAL mode enables concurrent reads/writes and improves write speed
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 3000)?;
        conn.pragma_update(None, "secure_delete", "ON")?;

        migrate(&mut conn)?;

        Ok(Self { conn })
    }

    /// Get current stats about the repository.
    pub fn report(&self) -> Result<OpenReport> {
        let version: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        let total: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))?;
        let pinned: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM entries WHERE pinned = 1", [], |row| {
                    row.get(0)
                })?;
        Ok(OpenReport {
            total_entries: total as usize,
            pinned_entries: pinned as usize,
            schema_version: version,
        })
    }

    /// Record a newly captured entry.
    pub fn record(&mut self, entry: NewEntry, dedup: DedupMode) -> Result<RecordOutcome> {
        let (preview, _) = preview_of(&entry.text);
        let char_count = entry.text.chars().count();
        let hash_bytes = &entry.hash.0[..];

        let tx = self.conn.transaction()?;

        // Check deduplication
        let existing: Option<(i64, i64)> = match dedup {
            DedupMode::LatestOnly => {
                // Check if the most recent entry has the same hash and text
                tx.query_row(
                    "SELECT id, copied_at FROM entries ORDER BY copied_at DESC, id DESC LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?
                .filter(|(id, _)| {
                    let matches_hash: bool = tx
                        .query_row(
                            "SELECT 1 FROM entries WHERE id = ?1 AND hash = ?2",
                            params![id, hash_bytes],
                            |_| Ok(true),
                        )
                        .optional()
                        .unwrap_or(None)
                        .unwrap_or(false);
                    matches_hash
                })
            }
            DedupMode::Anywhere => {
                // Check if any entry has the exact same hash and text
                tx.query_row(
                    "SELECT id, copied_at FROM entries WHERE hash = ?1 AND text = ?2 ORDER BY copied_at DESC, id DESC LIMIT 1",
                    params![hash_bytes, &entry.text],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?
            }
        };

        if let Some((existing_id, _)) = existing {
            // Update copied_at and source_app if available to move it to the top
            tx.execute(
                "UPDATE entries SET copied_at = ?1, source_app = coalesce(?2, source_app) WHERE id = ?3",
                params![entry.copied_at, entry.source_app, existing_id],
            )?;
            tx.commit()?;
            return Ok(RecordOutcome::Deduplicated {
                id: EntryId(existing_id),
                moved_to_top: true,
            });
        }

        // Insert new entry
        tx.execute(
            r#"
            INSERT INTO entries (text, preview, char_count, content_type, hash, copied_at, pinned, source_app)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)
            "#,
            params![
                entry.text,
                preview,
                char_count as i64,
                entry.kind.as_str(),
                hash_bytes,
                entry.copied_at,
                entry.source_app,
            ],
        )?;

        let id = tx.last_insert_rowid();
        tx.commit()?;

        Ok(RecordOutcome::Inserted(EntryId(id)))
    }

    /// List entries matching query criteria.
    pub fn list(&self, query: &HistoryQuery) -> Result<Vec<EntrySummary>> {
        let limit = if query.limit == 0 { 50 } else { query.limit };
        let offset = query.offset;

        let mut stmt;
        let mut summaries = Vec::new();

        if let Some(ref term) = query.search_term {
            let term = term.trim();
            if !term.is_empty() {
                // Case-insensitive substring search in SQLite using LIKE
                // We escape special LIKE characters (% and _)
                let escaped = term
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_");
                let pattern = format!("%{}%", escaped);

                stmt = self.conn.prepare(
                    r#"
                    SELECT id, preview, char_count, content_type, copied_at, pinned, source_app
                    FROM entries
                    WHERE text LIKE ?1 ESCAPE '\'
                    ORDER BY pinned DESC, copied_at DESC, id DESC
                    LIMIT ?2 OFFSET ?3
                    "#,
                )?;
                let rows = stmt.query_map(
                    params![pattern, limit as i64, offset as i64],
                    row_to_summary,
                )?;
                for row in rows {
                    summaries.push(row?);
                }
                return Ok(summaries);
            }
        }

        stmt = self.conn.prepare(
            r#"
            SELECT id, preview, char_count, content_type, copied_at, pinned, source_app
            FROM entries
            ORDER BY pinned DESC, copied_at DESC, id DESC
            LIMIT ?1 OFFSET ?2
            "#,
        )?;
        let rows = stmt.query_map(params![limit as i64, offset as i64], row_to_summary)?;
        for row in rows {
            summaries.push(row?);
        }

        Ok(summaries)
    }

    /// Retrieve the full text of an entry by ID.
    pub fn get_text(&self, id: EntryId) -> Result<Option<String>> {
        let text = self
            .conn
            .query_row(
                "SELECT text FROM entries WHERE id = ?1",
                params![id.0],
                |row| row.get(0),
            )
            .optional()?;
        Ok(text)
    }

    /// Toggle the pinned status of an entry. Returns the new pinned state.
    pub fn toggle_pin(&mut self, id: EntryId) -> Result<Option<bool>> {
        let tx = self.conn.transaction()?;
        let current: Option<bool> = tx
            .query_row(
                "SELECT pinned == 1 FROM entries WHERE id = ?1",
                params![id.0],
                |row| row.get(0),
            )
            .optional()?;

        let Some(current_pinned) = current else {
            return Ok(None);
        };

        let new_state = !current_pinned;
        tx.execute(
            "UPDATE entries SET pinned = ?1 WHERE id = ?2",
            params![if new_state { 1 } else { 0 }, id.0],
        )?;
        tx.commit()?;
        Ok(Some(new_state))
    }

    /// Delete an individual entry by ID. Returns true if an entry was deleted.
    pub fn delete(&mut self, id: EntryId) -> Result<bool> {
        let count = self
            .conn
            .execute("DELETE FROM entries WHERE id = ?1", params![id.0])?;
        Ok(count > 0)
    }

    /// Checkpoint the WAL file and truncate it, ensuring deleted content is cleared from the write-ahead log.
    pub fn checkpoint(&self) -> Result<()> {
        self.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    /// Clear all unpinned entries. Returns the number of removed entries.
    pub fn clear_unpinned(&mut self) -> Result<usize> {
        let count = self
            .conn
            .execute("DELETE FROM entries WHERE pinned = 0", [])?;
        if count > 0 {
            let _ = self.checkpoint();
        }
        Ok(count)
    }

    /// Clear ALL entries, including pinned. Returns the number of removed entries.
    pub fn clear_all(&mut self) -> Result<usize> {
        let count = self.conn.execute("DELETE FROM entries", [])?;
        let _ = self.checkpoint();
        Ok(count)
    }

    /// Apply the retention policy to clean up old or surplus entries.
    /// Pinned entries are NEVER deleted by retention.
    /// Returns the total number of deleted entries.
    pub fn apply_retention(&mut self, policy: &RetentionPolicy, now_ms: i64) -> Result<usize> {
        let mut deleted = 0;
        let tx = self.conn.transaction()?;

        // 1. Delete entries older than age cutoff
        if let Some(cutoff) = policy.age_cutoff(now_ms) {
            let count = tx.execute(
                "DELETE FROM entries WHERE pinned = 0 AND copied_at < ?1",
                params![cutoff],
            )?;
            deleted += count;
        }

        // 2. Delete excess unpinned entries beyond max_entries
        if let Some(max) = policy.max_entries {
            // Find entries exceeding the threshold ordered newest first
            let count = tx.execute(
                r#"
                DELETE FROM entries
                WHERE id IN (
                    SELECT id FROM entries
                    WHERE pinned = 0
                    ORDER BY copied_at DESC, id DESC
                    LIMIT -1 OFFSET ?1
                )
                "#,
                params![max as i64],
            )?;
            deleted += count;
        }

        tx.commit()?;

        if deleted > 0 {
            let _ = self.checkpoint();
        }

        Ok(deleted)
    }

    /// Get total count of entries.
    pub fn count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))?;
        Ok(count as usize)
    }
}

fn row_to_summary(row: &Row) -> rusqlite::Result<EntrySummary> {
    let id: i64 = row.get(0)?;
    let preview: String = row.get(1)?;
    let char_count: i64 = row.get(2)?;
    let content_type_str: String = row.get(3)?;
    let copied_at: i64 = row.get(4)?;
    let pinned_int: i64 = row.get(5)?;
    let source_app: Option<String> = row.get(6)?;

    let char_count = char_count as usize;
    let truncated = preview.chars().count() < char_count;

    Ok(EntrySummary {
        id: EntryId(id),
        preview,
        truncated,
        char_count,
        kind: ContentKind::from_db(&content_type_str),
        copied_at,
        pinned: pinned_int == 1,
        source_app,
    })
}

trait OptionalExt<T> {
    fn optional(self) -> rusqlite::Result<Option<T>>;
}

impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> rusqlite::Result<Option<T>> {
        match self {
            Ok(val) => Ok(Some(val)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::capture::prepare;
    use crate::domain::settings::CaptureSettings;
    use std::time::Duration;

    fn sample_entry(text: &str, copied_at: i64) -> NewEntry {
        prepare(
            text.to_string(),
            &CaptureSettings::default(),
            Some("test_app.exe".to_string()),
            copied_at,
        )
        .expect("prepare failed")
    }

    #[test]
    fn secure_delete_is_enabled() {
        let repo = Repository::open_in_memory().expect("open failed");
        let secure_delete: i32 = repo
            .conn
            .pragma_query_value(None, "secure_delete", |row| row.get(0))
            .expect("pragma query failed");
        assert_eq!(secure_delete, 1);
    }

    #[test]
    fn open_and_migrate_in_memory() {
        let repo = Repository::open_in_memory().expect("open failed");
        let rep = repo.report().expect("report failed");
        assert_eq!(rep.total_entries, 0);
        assert_eq!(rep.pinned_entries, 0);
        assert_eq!(rep.schema_version, 1);
    }

    #[test]
    fn record_and_retrieve_entry() {
        let mut repo = Repository::open_in_memory().unwrap();
        let entry = sample_entry("Hello, clipboard!", 1000);
        let outcome = repo.record(entry, DedupMode::Anywhere).unwrap();

        let id = match outcome {
            RecordOutcome::Inserted(id) => id,
            _ => panic!("Expected Inserted"),
        };

        let text = repo.get_text(id).unwrap();
        assert_eq!(text.as_deref(), Some("Hello, clipboard!"));

        let list = repo.list(&HistoryQuery::default()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].preview, "Hello, clipboard!");
        assert_eq!(list[0].source_app.as_deref(), Some("test_app.exe"));
        assert!(!list[0].pinned);
    }

    #[test]
    fn deduplication_anywhere_moves_to_top() {
        let mut repo = Repository::open_in_memory().unwrap();
        let e1 = sample_entry("First", 1000);
        let e2 = sample_entry("Second", 2000);
        let e3 = sample_entry("First", 3000);

        let id1 = match repo.record(e1, DedupMode::Anywhere).unwrap() {
            RecordOutcome::Inserted(id) => id,
            _ => panic!(),
        };
        let id2 = match repo.record(e2, DedupMode::Anywhere).unwrap() {
            RecordOutcome::Inserted(id) => id,
            _ => panic!(),
        };
        let outcome = repo.record(e3, DedupMode::Anywhere).unwrap();

        assert_eq!(
            outcome,
            RecordOutcome::Deduplicated {
                id: id1,
                moved_to_top: true
            }
        );
        assert_eq!(repo.count().unwrap(), 2);

        // In query results, id1 should now be first because copied_at is 3000 > 2000
        let list = repo.list(&HistoryQuery::default()).unwrap();
        assert_eq!(list[0].id, id1);
        assert_eq!(list[1].id, id2);
    }

    #[test]
    fn deduplication_latest_only() {
        let mut repo = Repository::open_in_memory().unwrap();
        let e1 = sample_entry("A", 1000);
        let e2 = sample_entry("B", 2000);
        let e3 = sample_entry("A", 3000);

        repo.record(e1, DedupMode::LatestOnly).unwrap();
        repo.record(e2, DedupMode::LatestOnly).unwrap();
        let outcome = repo.record(e3, DedupMode::LatestOnly).unwrap();

        // Since latest was "B", "A" should be inserted newly
        assert!(matches!(outcome, RecordOutcome::Inserted(_)));
        assert_eq!(repo.count().unwrap(), 3);
    }

    #[test]
    fn pin_and_unpin() {
        let mut repo = Repository::open_in_memory().unwrap();
        let id1 = match repo
            .record(sample_entry("First", 1000), DedupMode::Anywhere)
            .unwrap()
        {
            RecordOutcome::Inserted(id) => id,
            _ => panic!(),
        };
        let id2 = match repo
            .record(sample_entry("Second", 2000), DedupMode::Anywhere)
            .unwrap()
        {
            RecordOutcome::Inserted(id) => id,
            _ => panic!(),
        };

        // Pin the older one
        let pinned = repo.toggle_pin(id1).unwrap();
        assert_eq!(pinned, Some(true));

        // Pinned entries come first in list
        let list = repo.list(&HistoryQuery::default()).unwrap();
        assert_eq!(list[0].id, id1);
        assert!(list[0].pinned);
        assert_eq!(list[1].id, id2);
        assert!(!list[1].pinned);

        // Toggle back
        let pinned = repo.toggle_pin(id1).unwrap();
        assert_eq!(pinned, Some(false));
    }

    #[test]
    fn delete_and_clear() {
        let mut repo = Repository::open_in_memory().unwrap();
        let id1 = match repo
            .record(sample_entry("Pinned", 1000), DedupMode::Anywhere)
            .unwrap()
        {
            RecordOutcome::Inserted(id) => id,
            _ => panic!(),
        };
        let id2 = match repo
            .record(sample_entry("Unpinned", 2000), DedupMode::Anywhere)
            .unwrap()
        {
            RecordOutcome::Inserted(id) => id,
            _ => panic!(),
        };
        repo.toggle_pin(id1).unwrap();

        // Clear unpinned
        let deleted = repo.clear_unpinned().unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(repo.count().unwrap(), 1);
        assert_eq!(repo.get_text(id2).unwrap(), None);
        assert_eq!(repo.get_text(id1).unwrap(), Some("Pinned".to_string()));

        // Clear all
        let deleted = repo.clear_all().unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(repo.count().unwrap(), 0);
    }

    #[test]
    fn retention_policy_enforcement() {
        let mut repo = Repository::open_in_memory().unwrap();
        // Insert 5 entries, pin the oldest
        let id0 = match repo
            .record(sample_entry("0", 100), DedupMode::Anywhere)
            .unwrap()
        {
            RecordOutcome::Inserted(id) => id,
            _ => panic!(),
        };
        repo.toggle_pin(id0).unwrap();

        for i in 1..=4 {
            repo.record(
                sample_entry(&format!("{i}"), 100 + i * 100),
                DedupMode::Anywhere,
            )
            .unwrap();
        }
        assert_eq!(repo.count().unwrap(), 5);

        // Keep at most 2 unpinned entries
        let policy = RetentionPolicy {
            max_entries: Some(2),
            max_age: None,
        };
        let deleted = repo.apply_retention(&policy, 1000).unwrap();
        assert_eq!(deleted, 2);
        // Pinned 0 is kept, plus newest 2 (entries 3 and 4)
        assert_eq!(repo.count().unwrap(), 3);
        assert!(repo.get_text(id0).unwrap().is_some());

        // Now test age retention
        let age_policy = RetentionPolicy {
            max_entries: None,
            max_age: Some(Duration::from_millis(500)),
        };
        // Now is 1000. Cutoff is 500. Entries with copied_at < 500 should be removed (unless pinned).
        // id0 copied_at is 100, but pinned!
        // entries 3 and 4 have copied_at 400 and 500.
        // entry 3 (400) is < 500, so it will be deleted.
        let deleted = repo.apply_retention(&age_policy, 1000).unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(repo.count().unwrap(), 2);
        assert!(repo.get_text(id0).unwrap().is_some());
    }

    #[test]
    fn search_case_insensitive() {
        let mut repo = Repository::open_in_memory().unwrap();
        repo.record(sample_entry("Hello World", 1000), DedupMode::Anywhere)
            .unwrap();
        repo.record(sample_entry("Rust Programming", 2000), DedupMode::Anywhere)
            .unwrap();
        repo.record(sample_entry("hello there", 3000), DedupMode::Anywhere)
            .unwrap();

        let q = HistoryQuery {
            search_term: Some("HELLO".to_string()),
            limit: 10,
            offset: 0,
        };
        let results = repo.list(&q).unwrap();
        assert_eq!(results.len(), 2);
    }
}
