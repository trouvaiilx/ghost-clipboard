//! Asynchronous storage worker.
//!
//! Owns the SQLite `Repository` on a dedicated background thread.
//! Keeps all database operations and disk I/O completely off the GUI thread.

use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use tokio::sync::{mpsc, oneshot};

use crate::domain::entry::{EntryId, EntrySummary, NewEntry};
use crate::domain::settings::{DedupMode, RetentionPolicy};
use crate::storage::repository::{HistoryQuery, OpenReport, RecordOutcome, Repository};
use crate::storage::{Result, StorageError};

const STORAGE_QUEUE_CAPACITY: usize = 128;

pub type Reply<T> = oneshot::Sender<Result<T>>;

enum Command {
    Record {
        entry: NewEntry,
        dedup: DedupMode,
        reply: Reply<RecordOutcome>,
    },
    List {
        query: HistoryQuery,
        reply: Reply<Vec<EntrySummary>>,
    },
    GetText {
        id: EntryId,
        reply: Reply<Option<String>>,
    },
    TogglePin {
        id: EntryId,
        reply: Reply<Option<bool>>,
    },
    Delete {
        id: EntryId,
        reply: Reply<bool>,
    },
    ClearUnpinned {
        reply: Reply<usize>,
    },
    ClearAll {
        reply: Reply<usize>,
    },
    ApplyRetention {
        policy: RetentionPolicy,
        now_ms: i64,
        reply: Reply<usize>,
    },
    Report {
        reply: Reply<OpenReport>,
    },
}

/// A cloneable handle to dispatch commands to the background storage thread.
#[derive(Clone)]
pub struct StorageHandle {
    tx: mpsc::Sender<Command>,
}

impl StorageHandle {
    pub async fn record(&self, entry: NewEntry, dedup: DedupMode) -> Result<RecordOutcome> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Command::Record {
                entry,
                dedup,
                reply,
            })
            .await
            .map_err(|_| StorageError::Stopped)?;
        rx.await.map_err(|_| StorageError::Stopped)?
    }

    pub async fn list(&self, query: HistoryQuery) -> Result<Vec<EntrySummary>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Command::List { query, reply })
            .await
            .map_err(|_| StorageError::Stopped)?;
        rx.await.map_err(|_| StorageError::Stopped)?
    }

    pub async fn get_text(&self, id: EntryId) -> Result<Option<String>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Command::GetText { id, reply })
            .await
            .map_err(|_| StorageError::Stopped)?;
        rx.await.map_err(|_| StorageError::Stopped)?
    }

    pub async fn toggle_pin(&self, id: EntryId) -> Result<Option<bool>> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Command::TogglePin { id, reply })
            .await
            .map_err(|_| StorageError::Stopped)?;
        rx.await.map_err(|_| StorageError::Stopped)?
    }

    pub async fn delete(&self, id: EntryId) -> Result<bool> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Command::Delete { id, reply })
            .await
            .map_err(|_| StorageError::Stopped)?;
        rx.await.map_err(|_| StorageError::Stopped)?
    }

    pub async fn clear_unpinned(&self) -> Result<usize> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Command::ClearUnpinned { reply })
            .await
            .map_err(|_| StorageError::Stopped)?;
        rx.await.map_err(|_| StorageError::Stopped)?
    }

    pub async fn clear_all(&self) -> Result<usize> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Command::ClearAll { reply })
            .await
            .map_err(|_| StorageError::Stopped)?;
        rx.await.map_err(|_| StorageError::Stopped)?
    }

    pub async fn apply_retention(&self, policy: RetentionPolicy, now_ms: i64) -> Result<usize> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Command::ApplyRetention {
                policy,
                now_ms,
                reply,
            })
            .await
            .map_err(|_| StorageError::Stopped)?;
        rx.await.map_err(|_| StorageError::Stopped)?
    }

    pub async fn report(&self) -> Result<OpenReport> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Command::Report { reply })
            .await
            .map_err(|_| StorageError::Stopped)?;
        rx.await.map_err(|_| StorageError::Stopped)?
    }
}

pub struct Storage {
    handle: StorageHandle,
    thread: Option<JoinHandle<()>>,
}

impl Storage {
    /// Start the storage thread with a database file at `path`.
    pub fn spawn<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_buf = path.as_ref().to_path_buf();
        Self::spawn_internal(move || Repository::open(&path_buf))
    }

    /// Start the storage thread with an in-memory database (for tests).
    pub fn spawn_in_memory() -> Result<Self> {
        Self::spawn_internal(Repository::open_in_memory)
    }

    fn spawn_internal<F>(open_fn: F) -> Result<Self>
    where
        F: FnOnce() -> Result<Repository> + Send + 'static,
    {
        let (tx, mut rx) = mpsc::channel(STORAGE_QUEUE_CAPACITY);
        let (init_tx, init_rx) = std::sync::mpsc::sync_channel(1);

        let thread = std::thread::Builder::new()
            .name("ghostclipboard-storage".into())
            .spawn(move || {
                let mut repo = match open_fn() {
                    Ok(r) => {
                        let _ = init_tx.send(Ok(()));
                        r
                    }
                    Err(e) => {
                        let _ = init_tx.send(Err(e));
                        return;
                    }
                };

                while let Some(cmd) = rx.blocking_recv() {
                    match cmd {
                        Command::Record {
                            entry,
                            dedup,
                            reply,
                        } => {
                            let _ = reply.send(repo.record(entry, dedup));
                        }
                        Command::List { query, reply } => {
                            let _ = reply.send(repo.list(&query));
                        }
                        Command::GetText { id, reply } => {
                            let _ = reply.send(repo.get_text(id));
                        }
                        Command::TogglePin { id, reply } => {
                            let _ = reply.send(repo.toggle_pin(id));
                        }
                        Command::Delete { id, reply } => {
                            let _ = reply.send(repo.delete(id));
                        }
                        Command::ClearUnpinned { reply } => {
                            let _ = reply.send(repo.clear_unpinned());
                        }
                        Command::ClearAll { reply } => {
                            let _ = reply.send(repo.clear_all());
                        }
                        Command::ApplyRetention {
                            policy,
                            now_ms,
                            reply,
                        } => {
                            let _ = reply.send(repo.apply_retention(&policy, now_ms));
                        }
                        Command::Report { reply } => {
                            let _ = reply.send(repo.report());
                        }
                    }
                }

                tracing::debug!("storage worker thread stopped");
            })
            .map_err(StorageError::Io)?;

        match init_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                handle: StorageHandle { tx },
                thread: Some(thread),
            }),
            Ok(Err(err)) => {
                let _ = thread.join();
                Err(err)
            }
            Err(_) => {
                let _ = thread.join();
                Err(StorageError::Stopped)
            }
        }
    }

    pub fn handle(&self) -> StorageHandle {
        self.handle.clone()
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        if let Some(_thread) = self.thread.take() {
            // Drop our local handle so rx will eventually terminate if no other handles exist
            drop(self.handle.tx.clone());
            // We don't block indefinitely on join in drop, but dropping handle triggers rx to close
        }
    }
}

/// Returns the standard default application directory for GhostClipboard.
/// Typically `%APPDATA%\GhostClipboard` on Windows.
pub fn default_data_dir() -> Result<PathBuf> {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    let app_dir = base.join("GhostClipboard");
    if !app_dir.exists() {
        std::fs::create_dir_all(&app_dir)?;
    }
    Ok(app_dir)
}

/// Returns the default path to the SQLite database file.
pub fn default_db_path() -> Result<PathBuf> {
    Ok(default_data_dir()?.join("history.db"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::capture::prepare;
    use crate::domain::settings::CaptureSettings;

    #[tokio::test]
    async fn storage_worker_crud() {
        let storage = Storage::spawn_in_memory().unwrap();
        let handle = storage.handle();

        let entry = prepare(
            "Worker test text".to_string(),
            &CaptureSettings::default(),
            Some("worker.exe".to_string()),
            100,
        )
        .unwrap();

        let outcome = handle.record(entry, DedupMode::Anywhere).await.unwrap();
        let id = match outcome {
            RecordOutcome::Inserted(id) => id,
            _ => panic!("Expected Inserted"),
        };

        let text = handle.get_text(id).await.unwrap();
        assert_eq!(text.as_deref(), Some("Worker test text"));

        let list = handle.list(HistoryQuery::default()).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);

        let pinned = handle.toggle_pin(id).await.unwrap();
        assert_eq!(pinned, Some(true));

        let deleted = handle.delete(id).await.unwrap();
        assert!(deleted);

        let list_empty = handle.list(HistoryQuery::default()).await.unwrap();
        assert_eq!(list_empty.len(), 0);
    }
}
