//! SQLite persistence.
//!
//! * [`Repository`] is the synchronous, fully testable data-access layer.
//! * [`Storage`] runs a `Repository` on a dedicated thread so callers on the
//!   GUI thread never wait on disk I/O.

pub mod migrations;
pub mod repository;
pub mod worker;

pub use repository::{HistoryQuery, OpenReport, RecordOutcome, Repository};
pub use worker::{Reply, Storage, StorageHandle, default_data_dir, default_db_path};

/// Storage failures. Messages describe *what* failed and never include clipboard
/// content (SQLite error text does not echo bound parameters).
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("the history database is damaged and could not be read: {0}")]
    Corrupt(String),
    #[error("the history database is locked by another process")]
    Locked,
    #[error("the disk is full, so the history database could not be written")]
    DiskFull,
    #[error(
        "the history database was created by a newer version of GhostClipboard (schema {found}, this version supports up to {supported})"
    )]
    UnsupportedVersion { found: i64, supported: i64 },
    #[error("database error: {0}")]
    Sqlite(String),
    #[error("could not access the data folder: {0}")]
    Io(#[from] std::io::Error),
    #[error("the storage worker is not running")]
    Stopped,
    #[error("the storage worker is too busy to accept more work")]
    Busy,
}

impl From<rusqlite::Error> for StorageError {
    fn from(error: rusqlite::Error) -> Self {
        use rusqlite::ErrorCode;
        match &error {
            rusqlite::Error::SqliteFailure(failure, _) => match failure.code {
                ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase => {
                    StorageError::Corrupt(error.to_string())
                }
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => StorageError::Locked,
                ErrorCode::DiskFull => StorageError::DiskFull,
                _ => StorageError::Sqlite(error.to_string()),
            },
            _ => StorageError::Sqlite(error.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, StorageError>;
