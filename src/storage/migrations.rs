//! Schema migrations.
//!
//! The schema version is stored in SQLite's `user_version` pragma. Each entry in
//! [`MIGRATIONS`] upgrades the database by one version and runs inside its own
//! transaction together with the version bump, so a failed migration leaves the
//! database untouched at the previous version.
//!
//! Rules for adding a migration: append only, never edit an existing entry.

use rusqlite::Connection;

use super::{Result, StorageError};

const MIGRATIONS: &[&str] = &[
    // v1: initial schema.
    //
    // * AUTOINCREMENT guarantees ids are never reused, so a stale id held by
    //   the UI can never point at a different, newer entry.
    // * `preview` and `char_count` let list queries avoid loading full text.
    // * `hash` is indexed (not unique): deduplication behaviour is a runtime
    //   policy, and "latest only" mode legitimately stores repeated text.
    r#"
    CREATE TABLE entries (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        text         TEXT    NOT NULL,
        preview      TEXT    NOT NULL,
        char_count   INTEGER NOT NULL,
        content_type TEXT    NOT NULL,
        hash         BLOB    NOT NULL,
        copied_at    INTEGER NOT NULL,
        pinned       INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
        source_app   TEXT
    );
    CREATE INDEX idx_entries_copied_at ON entries (copied_at DESC, id DESC);
    CREATE INDEX idx_entries_hash      ON entries (hash);
    CREATE INDEX idx_entries_pinned    ON entries (pinned, copied_at DESC, id DESC);
    "#,
];

/// The schema version this build produces.
pub const LATEST_VERSION: i64 = MIGRATIONS.len() as i64;

pub fn current_version(conn: &Connection) -> Result<i64> {
    Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

/// Bring `conn` up to [`LATEST_VERSION`], or fail without modifying a database
/// that was written by a newer version.
pub fn migrate(conn: &mut Connection) -> Result<()> {
    let version = current_version(conn)?;
    if version > LATEST_VERSION {
        return Err(StorageError::UnsupportedVersion {
            found: version,
            supported: LATEST_VERSION,
        });
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", index as i64 + 1)?;
        tx.commit()?;
        tracing::info!(version = index + 1, "applied database migration");
    }
    Ok(())
}
